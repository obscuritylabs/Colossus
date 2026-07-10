//! Strict YAML workflow validation and event-sourced durable run service.

use async_recursion::async_recursion;
use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, EventClassification, ExecutionContext, NewEvent, WorkflowDefinition,
    WorkflowRun, WorkflowStatus, WorkflowStep,
};
use colossus_ports::{EventJournal, StoreError, WorkflowRepository};
use futures::{StreamExt as _, TryStreamExt as _, stream};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};
use thiserror::Error;
use tokio::sync::Semaphore;
use uuid::Uuid;

const MAX_WORKFLOW_BYTES: usize = 1024 * 1024;
const MAX_CONCURRENCY: u32 = 64;
const MAX_STEP_BUDGET: u32 = 10_000;
const MAX_FOREACH_ITEMS: u32 = 1_000;

/// Workflow validation or durable execution failure.
#[derive(Debug, Error)]
pub enum WorkflowError {
    /// Definition violates the strict workflow contract.
    #[error("invalid workflow definition: {0}")]
    InvalidDefinition(String),
    /// Inputs or outputs violate the declared JSON Schema.
    #[error("workflow schema validation failed: {0}")]
    Schema(String),
    /// Definition or run does not exist.
    #[error("workflow record not found: {0}")]
    NotFound(String),
    /// Run cannot perform the requested transition.
    #[error("invalid workflow transition: {0}")]
    InvalidTransition(String),
    /// A policy-controlled effect failed.
    #[error("workflow effect failed: {0}")]
    Effect(String),
    /// The effect may have occurred; the run must be interrupted, not retried.
    #[error("workflow effect outcome unknown: {0}")]
    OutcomeUnknown(String),
    /// Canonical journal or repository failure.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Parsed definition plus trust-pinned raw content hash.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedWorkflow {
    /// Strict definition.
    pub definition: WorkflowDefinition,
    /// SHA-256 of the exact UTF-8 YAML bytes.
    pub content_hash: String,
}

/// Validate strict YAML, structural limits, step identifiers, expressions, and schemas.
pub fn validate_definition(yaml: &str) -> Result<ValidatedWorkflow, WorkflowError> {
    if yaml.len() > MAX_WORKFLOW_BYTES {
        return Err(WorkflowError::InvalidDefinition(format!(
            "definition exceeds {MAX_WORKFLOW_BYTES} bytes"
        )));
    }
    if yaml.contains("!!") || yaml.contains("&") || yaml.contains("*") {
        return Err(WorkflowError::InvalidDefinition(
            "YAML tags, anchors, and aliases are prohibited".into(),
        ));
    }
    let definition: WorkflowDefinition = serde_saphyr::from_str(yaml)
        .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?;
    if definition.api_version != "colossus.dev/v1alpha1" || definition.kind != "Workflow" {
        return Err(WorkflowError::InvalidDefinition(
            "apiVersion must be colossus.dev/v1alpha1 and kind must be Workflow".into(),
        ));
    }
    if definition.metadata.name.is_empty()
        || definition.metadata.version.is_empty()
        || definition.metadata.description.is_empty()
    {
        return Err(WorkflowError::InvalidDefinition(
            "metadata name, version, and description are required".into(),
        ));
    }
    if !valid_name(&definition.metadata.name) {
        return Err(WorkflowError::InvalidDefinition(
            "workflow name must use lowercase letters, digits, dots, or hyphens".into(),
        ));
    }
    if !(1..=MAX_CONCURRENCY).contains(&definition.max_concurrency) {
        return Err(WorkflowError::InvalidDefinition(format!(
            "maxConcurrency must be between 1 and {MAX_CONCURRENCY}"
        )));
    }
    if !(1..=MAX_STEP_BUDGET).contains(&definition.step_budget) {
        return Err(WorkflowError::InvalidDefinition(format!(
            "stepBudget must be between 1 and {MAX_STEP_BUDGET}"
        )));
    }
    if definition.steps.is_empty() {
        return Err(WorkflowError::InvalidDefinition(
            "at least one root step is required".into(),
        ));
    }
    jsonschema::validator_for(&definition.inputs)
        .map_err(|error| WorkflowError::Schema(format!("invalid input schema: {error}")))?;
    jsonschema::validator_for(&definition.outputs)
        .map_err(|error| WorkflowError::Schema(format!("invalid output schema: {error}")))?;
    let mut ids = BTreeSet::new();
    let step_count = validate_steps(&definition.steps, definition.max_concurrency, &mut ids)?;
    reject_direct_workflow_cycle(
        &definition.steps,
        &definition.metadata.name,
        &definition.metadata.version,
    )?;
    if step_count > definition.step_budget {
        return Err(WorkflowError::InvalidDefinition(format!(
            "definition contains {step_count} steps but budget is {}",
            definition.step_budget
        )));
    }
    let mut capabilities = BTreeSet::new();
    for capability in &definition.capabilities {
        if capability.is_empty() || !capabilities.insert(capability) {
            return Err(WorkflowError::InvalidDefinition(
                "capabilities must be non-empty and unique".into(),
            ));
        }
    }
    Ok(ValidatedWorkflow {
        definition,
        content_hash: hex::encode(Sha256::digest(yaml.as_bytes())),
    })
}

fn reject_direct_workflow_cycle(
    steps: &[WorkflowStep],
    name: &str,
    version: &str,
) -> Result<(), WorkflowError> {
    for step in steps {
        match step {
            WorkflowStep::Workflow {
                workflow,
                version: called_version,
                ..
            } if workflow == name && called_version == version => {
                return Err(WorkflowError::InvalidDefinition(
                    "workflow cannot directly call its own name and version".into(),
                ));
            }
            WorkflowStep::Condition {
                then, otherwise, ..
            } => {
                reject_direct_workflow_cycle(then, name, version)?;
                reject_direct_workflow_cycle(otherwise, name, version)?;
            }
            WorkflowStep::Parallel { branches, .. } => {
                for branch in branches {
                    reject_direct_workflow_cycle(branch, name, version)?;
                }
            }
            WorkflowStep::Foreach { steps, .. } => {
                reject_direct_workflow_cycle(steps, name, version)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn valid_name(name: &str) -> bool {
    name.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
    })
}

fn validate_steps(
    steps: &[WorkflowStep],
    workflow_concurrency: u32,
    ids: &mut BTreeSet<String>,
) -> Result<u32, WorkflowError> {
    let mut count = 0_u32;
    for step in steps {
        count = count.saturating_add(1);
        let id = step_id(step);
        if !valid_step_id(id) || !ids.insert(id.to_owned()) {
            return Err(WorkflowError::InvalidDefinition(format!(
                "step id {id:?} is invalid or duplicated"
            )));
        }
        match step {
            WorkflowStep::Agent { idempotency, .. } | WorkflowStep::Tool { idempotency, .. } => {
                if idempotency.as_deref().is_some_and(str::is_empty) {
                    return Err(WorkflowError::InvalidDefinition(
                        "idempotency strategy cannot be empty".into(),
                    ));
                }
            }
            WorkflowStep::Workflow {
                workflow, version, ..
            } if workflow.is_empty() || version.is_empty() => {
                return Err(WorkflowError::InvalidDefinition(
                    "subworkflow name and version are required".into(),
                ));
            }
            WorkflowStep::Condition {
                expression,
                then,
                otherwise,
                ..
            } => {
                Condition::parse(expression)?;
                count = count
                    .saturating_add(validate_steps(then, workflow_concurrency, ids)?)
                    .saturating_add(validate_steps(otherwise, workflow_concurrency, ids)?);
            }
            WorkflowStep::Parallel {
                branches,
                max_concurrency,
                ..
            } => {
                if branches.is_empty()
                    || *max_concurrency == 0
                    || *max_concurrency > workflow_concurrency
                {
                    return Err(WorkflowError::InvalidDefinition(
                        "parallel branches must be non-empty and locally bounded".into(),
                    ));
                }
                for branch in branches {
                    if branch.is_empty() {
                        return Err(WorkflowError::InvalidDefinition(
                            "parallel branches cannot be empty".into(),
                        ));
                    }
                    count =
                        count.saturating_add(validate_steps(branch, workflow_concurrency, ids)?);
                }
            }
            WorkflowStep::Foreach {
                items,
                max_items,
                steps,
                ..
            } => {
                if !items.starts_with('/')
                    || *max_items == 0
                    || *max_items > MAX_FOREACH_ITEMS
                    || steps.is_empty()
                {
                    return Err(WorkflowError::InvalidDefinition(
                        "foreach needs a JSON pointer, 1..=1000 item limit, and steps".into(),
                    ));
                }
                count = count.saturating_add(validate_steps(steps, workflow_concurrency, ids)?);
            }
            WorkflowStep::WaitForInput { schema, .. } => {
                jsonschema::validator_for(schema).map_err(|error| {
                    WorkflowError::Schema(format!("invalid wait_for_input schema: {error}"))
                })?;
            }
            WorkflowStep::Workflow { .. }
            | WorkflowStep::Approval { .. }
            | WorkflowStep::Emit { .. } => {}
        }
    }
    Ok(count)
}

fn step_id(step: &WorkflowStep) -> &str {
    match step {
        WorkflowStep::Agent { id, .. }
        | WorkflowStep::Tool { id, .. }
        | WorkflowStep::Workflow { id, .. }
        | WorkflowStep::Approval { id, .. }
        | WorkflowStep::Condition { id, .. }
        | WorkflowStep::Parallel { id, .. }
        | WorkflowStep::Foreach { id, .. }
        | WorkflowStep::WaitForInput { id, .. }
        | WorkflowStep::Emit { id, .. } => id,
    }
}

fn valid_step_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// Restricted non-executable workflow condition.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition(Expr);

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Exists(String),
    Compare(Operand, Compare, Operand),
    Not(Box<Self>),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
}

#[derive(Clone, Debug, PartialEq)]
enum Operand {
    Pointer(String),
    Literal(Value),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Compare {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Exists,
    Pointer(String),
    Literal(Value),
    LParen,
    RParen,
    Not,
    And,
    Or,
    Compare(Compare),
}

impl Condition {
    /// Parse and validate the entire restricted expression.
    pub fn parse(source: &str) -> Result<Self, WorkflowError> {
        let tokens = tokenize(source)?;
        let mut parser = Parser {
            tokens,
            position: 0,
        };
        let expression = parser.parse_or()?;
        if parser.position != parser.tokens.len() {
            return Err(WorkflowError::InvalidDefinition(
                "condition has trailing tokens".into(),
            ));
        }
        Ok(Self(expression))
    }

    /// Evaluate against a bounded JSON workflow context.
    pub fn evaluate(&self, context: &Value) -> bool {
        evaluate_expr(&self.0, context)
    }
}

fn tokenize(source: &str) -> Result<Vec<Token>, WorkflowError> {
    let chars = source.as_bytes();
    let mut position = 0;
    let mut tokens = Vec::new();
    while position < chars.len() {
        if chars[position].is_ascii_whitespace() {
            position += 1;
            continue;
        }
        let tail = &source[position..];
        let (token, consumed) = if tail.starts_with("exists") {
            (Token::Exists, 6)
        } else if tail.starts_with("&&") {
            (Token::And, 2)
        } else if tail.starts_with("||") {
            (Token::Or, 2)
        } else if tail.starts_with("==") {
            (Token::Compare(Compare::Eq), 2)
        } else if tail.starts_with("!=") {
            (Token::Compare(Compare::Ne), 2)
        } else if tail.starts_with(">=") {
            (Token::Compare(Compare::Ge), 2)
        } else if tail.starts_with("<=") {
            (Token::Compare(Compare::Le), 2)
        } else if tail.starts_with('!') {
            (Token::Not, 1)
        } else if tail.starts_with('>') {
            (Token::Compare(Compare::Gt), 1)
        } else if tail.starts_with('<') {
            (Token::Compare(Compare::Lt), 1)
        } else if tail.starts_with('(') {
            (Token::LParen, 1)
        } else if tail.starts_with(')') {
            (Token::RParen, 1)
        } else if tail.starts_with('/') {
            let length = tail
                .find(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '(' | ')' | '!' | '=' | '<' | '>' | '&' | '|')
                })
                .unwrap_or(tail.len());
            (Token::Pointer(tail[..length].into()), length)
        } else {
            let length = if tail.starts_with('"') {
                json_string_length(tail)?
            } else {
                tail.find(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '(' | ')' | '!' | '=' | '<' | '>' | '&' | '|')
                })
                .unwrap_or(tail.len())
            };
            let literal = serde_json::from_str(&tail[..length]).map_err(|_| {
                WorkflowError::InvalidDefinition(format!(
                    "condition literal {:?} is not JSON",
                    &tail[..length]
                ))
            })?;
            (Token::Literal(literal), length)
        };
        tokens.push(token);
        position += consumed;
    }
    if tokens.is_empty() {
        return Err(WorkflowError::InvalidDefinition(
            "condition cannot be empty".into(),
        ));
    }
    Ok(tokens)
}

fn json_string_length(source: &str) -> Result<usize, WorkflowError> {
    let mut escaped = false;
    for (index, character) in source.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return Ok(index + 1);
        }
    }
    Err(WorkflowError::InvalidDefinition(
        "unterminated JSON string in condition".into(),
    ))
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn parse_or(&mut self) -> Result<Expr, WorkflowError> {
        let mut expression = self.parse_and()?;
        while self.consume(&Token::Or) {
            expression = Expr::Or(Box::new(expression), Box::new(self.parse_and()?));
        }
        Ok(expression)
    }

    fn parse_and(&mut self) -> Result<Expr, WorkflowError> {
        let mut expression = self.parse_unary()?;
        while self.consume(&Token::And) {
            expression = Expr::And(Box::new(expression), Box::new(self.parse_unary()?));
        }
        Ok(expression)
    }

    fn parse_unary(&mut self) -> Result<Expr, WorkflowError> {
        if self.consume(&Token::Not) {
            return Ok(Expr::Not(Box::new(self.parse_unary()?)));
        }
        if self.consume(&Token::LParen) {
            let expression = self.parse_or()?;
            if !self.consume(&Token::RParen) {
                return Err(WorkflowError::InvalidDefinition(
                    "condition is missing a closing parenthesis".into(),
                ));
            }
            return Ok(expression);
        }
        if self.consume(&Token::Exists) {
            if !self.consume(&Token::LParen) {
                return Err(WorkflowError::InvalidDefinition(
                    "exists requires parentheses".into(),
                ));
            }
            let pointer = match self.next() {
                Some(Token::Pointer(pointer)) => pointer,
                _ => {
                    return Err(WorkflowError::InvalidDefinition(
                        "exists requires a JSON pointer".into(),
                    ));
                }
            };
            if !self.consume(&Token::RParen) {
                return Err(WorkflowError::InvalidDefinition(
                    "exists is missing a closing parenthesis".into(),
                ));
            }
            return Ok(Expr::Exists(pointer));
        }
        let left = self.parse_operand()?;
        let comparison = match self.next() {
            Some(Token::Compare(comparison)) => comparison,
            _ => {
                return Err(WorkflowError::InvalidDefinition(
                    "conditions must use exists or an explicit comparison".into(),
                ));
            }
        };
        let right = self.parse_operand()?;
        Ok(Expr::Compare(left, comparison, right))
    }

    fn parse_operand(&mut self) -> Result<Operand, WorkflowError> {
        match self.next() {
            Some(Token::Pointer(pointer)) => Ok(Operand::Pointer(pointer)),
            Some(Token::Literal(value)) => Ok(Operand::Literal(value)),
            _ => Err(WorkflowError::InvalidDefinition(
                "condition comparison operand is missing".into(),
            )),
        }
    }

    fn consume(&mut self, expected: &Token) -> bool {
        if self.tokens.get(self.position) == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += usize::from(token.is_some());
        token
    }
}

fn operand_value(operand: &Operand, context: &Value) -> Option<Value> {
    match operand {
        Operand::Pointer(pointer) => context.pointer(pointer).cloned(),
        Operand::Literal(value) => Some(value.clone()),
    }
}

fn evaluate_expr(expression: &Expr, context: &Value) -> bool {
    match expression {
        Expr::Exists(pointer) => context.pointer(pointer).is_some(),
        Expr::Not(expression) => !evaluate_expr(expression, context),
        Expr::And(left, right) => evaluate_expr(left, context) && evaluate_expr(right, context),
        Expr::Or(left, right) => evaluate_expr(left, context) || evaluate_expr(right, context),
        Expr::Compare(left, comparison, right) => {
            let (Some(left), Some(right)) =
                (operand_value(left, context), operand_value(right, context))
            else {
                return false;
            };
            match comparison {
                Compare::Eq => left == right,
                Compare::Ne => left != right,
                Compare::Gt | Compare::Ge | Compare::Lt | Compare::Le => {
                    compare_order(&left, &right, *comparison)
                }
            }
        }
    }
}

fn compare_order(left: &Value, right: &Value, comparison: Compare) -> bool {
    let ordering = match (left, right) {
        (Value::Number(left), Value::Number(right)) => left
            .as_f64()
            .zip(right.as_f64())
            .and_then(|(left, right)| left.partial_cmp(&right)),
        (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
        _ => None,
    };
    ordering.is_some_and(|ordering| match comparison {
        Compare::Gt => ordering.is_gt(),
        Compare::Ge => ordering.is_ge(),
        Compare::Lt => ordering.is_lt(),
        Compare::Le => ordering.is_le(),
        Compare::Eq | Compare::Ne => false,
    })
}

/// Event-sourced workflow definition and run repository.
pub struct EventSourcedWorkflowRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedWorkflowRepository {
    /// Create a repository over the canonical journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }
}

impl WorkflowRepository for EventSourcedWorkflowRepository {
    fn register(
        &self,
        definition: &WorkflowDefinition,
        content_hash: &str,
        provenance: &str,
    ) -> Result<(), StoreError> {
        let stream_id = format!(
            "workflow-definition:{}:{}",
            definition.metadata.name, definition.metadata.version
        );
        let existing = self.journal.read_stream(&stream_id)?;
        if let Some(last) = existing.last() {
            let payload = self.journal.decrypt_payload(last)?;
            if payload.get("content_hash").and_then(Value::as_str) == Some(content_hash) {
                return Ok(());
            }
        }
        let event_type = if existing.is_empty() {
            "workflow.definition.registered.v1"
        } else {
            "workflow.definition.changed.v1"
        };
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: u64::try_from(existing.len())
                .map_err(|error| StoreError::Adapter(error.to_string()))?,
            classification: EventClassification::Workflow,
            event_type: event_type.into(),
            actor: Actor {
                actor_type: ActorType::User,
                id: "workflow-registrar".into(),
            },
            context: ExecutionContext {
                correlation_id: Uuid::now_v7().to_string(),
                ..ExecutionContext::default()
            },
            payload: json!({
                "definition": definition,
                "content_hash": content_hash,
                "provenance": provenance,
                "trust_invalidated": !existing.is_empty(),
            }),
        })?;
        Ok(())
    }

    fn definition(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<(WorkflowDefinition, String)>, StoreError> {
        let stream_id = format!("workflow-definition:{name}:{version}");
        let Some(last) = self.journal.read_stream(&stream_id)?.last().cloned() else {
            return Ok(None);
        };
        let payload = self.journal.decrypt_payload(&last)?;
        let definition = serde_json::from_value(
            payload
                .get("definition")
                .cloned()
                .ok_or_else(|| StoreError::Verification("definition payload is absent".into()))?,
        )
        .map_err(|error| StoreError::Verification(error.to_string()))?;
        let content_hash = payload
            .get("content_hash")
            .and_then(Value::as_str)
            .ok_or_else(|| StoreError::Verification("definition hash is absent".into()))?;
        Ok(Some((definition, content_hash.into())))
    }

    fn run(&self, run_id: &str) -> Result<Option<WorkflowRun>, StoreError> {
        fold_run(self.journal.as_ref(), run_id)
    }

    fn runs(&self, limit: usize) -> Result<Vec<WorkflowRun>, StoreError> {
        let events = self.journal.read_global(1, usize::MAX)?;
        let mut run_ids = Vec::new();
        for event in events {
            if event.event_type == "workflow.run.started.v1"
                && let Some(run_id) = event.stream_id.strip_prefix("workflow-run:")
            {
                run_ids.push(run_id.to_owned());
            }
        }
        run_ids
            .into_iter()
            .rev()
            .take(limit)
            .map(|run_id| {
                fold_run(self.journal.as_ref(), &run_id)?.ok_or_else(|| {
                    StoreError::Verification(format!("run {run_id} start event is unreadable"))
                })
            })
            .collect()
    }
}

fn fold_run(journal: &dyn EventJournal, run_id: &str) -> Result<Option<WorkflowRun>, StoreError> {
    let events = journal.read_stream(&format!("workflow-run:{run_id}"))?;
    let Some(first) = events.first() else {
        return Ok(None);
    };
    let start = journal.decrypt_payload(first)?;
    let mut run = WorkflowRun {
        run_id: run_id.into(),
        workflow_name: string_field(&start, "workflow_name")?,
        workflow_version: string_field(&start, "workflow_version")?,
        workflow_hash: string_field(&start, "workflow_hash")?,
        status: WorkflowStatus::Running,
        inputs: start.get("inputs").cloned().unwrap_or(Value::Null),
        outputs: None,
        completed_steps: 0,
    };
    for event in events.iter().skip(1) {
        let payload = journal.decrypt_payload(event)?;
        match event.event_type.as_str() {
            "workflow.step.completed.v1" => {
                run.completed_steps = payload
                    .get("root_index")
                    .and_then(Value::as_u64)
                    .and_then(|index| u32::try_from(index.saturating_add(1)).ok())
                    .unwrap_or(run.completed_steps);
            }
            "workflow.run.waiting.v1" => run.status = WorkflowStatus::Waiting,
            "workflow.run.resumed.v1" => run.status = WorkflowStatus::Running,
            "workflow.run.completed.v1" => {
                run.status = WorkflowStatus::Completed;
                run.outputs = payload.get("outputs").cloned();
            }
            "workflow.run.failed.v1" => run.status = WorkflowStatus::Failed,
            "workflow.run.cancelled.v1" => run.status = WorkflowStatus::Cancelled,
            "workflow.run.interrupted.v1" => run.status = WorkflowStatus::Interrupted,
            _ => {}
        }
    }
    Ok(Some(run))
}

fn string_field(value: &Value, field: &str) -> Result<String, StoreError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| StoreError::Verification(format!("run field {field} is absent")))
}

/// Policy-controlled workflow effect request handed to the runtime gateway.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowEffect {
    /// Effect class (`agent`, `tool`, or `workflow`).
    pub kind: String,
    /// Action or registered tool/workflow name.
    pub action: String,
    /// Proposed logical content.
    pub content: Value,
    /// Optional explicit idempotency strategy.
    pub idempotency: Option<String>,
    /// Workflow run identifier.
    pub run_id: String,
    /// Workflow step identifier.
    pub step_id: String,
    /// Pinned definition hash.
    pub workflow_hash: String,
    /// One-based attempt number.
    pub attempt: u32,
}

/// Application/runtime bridge that routes every effectful step through the gateway.
#[async_trait]
pub trait WorkflowEffectRunner: Send + Sync {
    /// Run one policy-controlled step and return structured released output.
    async fn run(&self, effect: WorkflowEffect) -> Result<Value, WorkflowError>;
}

/// Durable workflow application API.
pub struct WorkflowService {
    journal: Arc<dyn EventJournal>,
    repository: Arc<dyn WorkflowRepository>,
    effects: Arc<dyn WorkflowEffectRunner>,
    event_writer: Mutex<()>,
}

impl WorkflowService {
    /// Compose the event-sourced service.
    pub fn new(
        journal: Arc<dyn EventJournal>,
        repository: Arc<dyn WorkflowRepository>,
        effects: Arc<dyn WorkflowEffectRunner>,
    ) -> Self {
        Self {
            journal,
            repository,
            effects,
            event_writer: Mutex::new(()),
        }
    }

    /// Validate and register an exact YAML definition and provenance.
    pub fn register_definition(
        &self,
        yaml: &str,
        provenance: &str,
    ) -> Result<ValidatedWorkflow, WorkflowError> {
        let validated = validate_definition(yaml)?;
        self.repository
            .register(&validated.definition, &validated.content_hash, provenance)?;
        Ok(validated)
    }

    /// Start and drive a run until it waits or reaches a terminal state.
    pub async fn start_run(
        &self,
        name: &str,
        version: &str,
        inputs: Value,
    ) -> Result<WorkflowRun, WorkflowError> {
        let (definition, workflow_hash) = self
            .repository
            .definition(name, version)?
            .ok_or_else(|| WorkflowError::NotFound(format!("{name}:{version}")))?;
        validate_instance(&definition.inputs, &inputs, "input")?;
        let run_id = Uuid::now_v7().to_string();
        self.append_run_event(
            &run_id,
            "workflow.run.started.v1",
            json!({
                "workflow_name": name,
                "workflow_version": version,
                "workflow_hash": workflow_hash,
                "inputs": inputs,
            }),
        )?;
        self.drive(&run_id, definition, workflow_hash, inputs, 0)
            .await?;
        self.get_run(&run_id)
    }

    /// Reconstruct one durable run.
    pub fn get_run(&self, run_id: &str) -> Result<WorkflowRun, WorkflowError> {
        self.repository
            .run(run_id)?
            .ok_or_else(|| WorkflowError::NotFound(run_id.into()))
    }

    /// List bounded durable runs.
    pub fn list_runs(&self, limit: usize) -> Result<Vec<WorkflowRun>, WorkflowError> {
        self.repository.runs(limit).map_err(Into::into)
    }

    /// Supply structured input to a waiting run and resume it.
    pub async fn provide_input(
        &self,
        run_id: &str,
        input: Value,
    ) -> Result<WorkflowRun, WorkflowError> {
        let run = self.get_run(run_id)?;
        if run.status != WorkflowStatus::Waiting {
            return Err(WorkflowError::InvalidTransition(format!(
                "run {run_id} is not waiting"
            )));
        }
        let (definition, current_hash) = self
            .repository
            .definition(&run.workflow_name, &run.workflow_version)?
            .ok_or_else(|| WorkflowError::NotFound(run.workflow_name.clone()))?;
        if current_hash != run.workflow_hash {
            return Err(WorkflowError::InvalidTransition(
                "workflow definition changed; pinned run trust is invalid".into(),
            ));
        }
        let root_index = usize::try_from(run.completed_steps)
            .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?;
        let step = definition.steps.get(root_index).ok_or_else(|| {
            WorkflowError::InvalidTransition("waiting step is outside the definition".into())
        })?;
        match step {
            WorkflowStep::WaitForInput { schema, .. } => {
                validate_instance(schema, &input, "workflow input response")?;
            }
            WorkflowStep::Approval { .. } => {
                let approved = input == Value::Bool(true)
                    || input.get("approved").and_then(Value::as_bool) == Some(true);
                if !approved {
                    return Err(WorkflowError::InvalidTransition(
                        "approval input must explicitly contain approved: true".into(),
                    ));
                }
            }
            _ => {
                return Err(WorkflowError::InvalidTransition(
                    "the waiting root step does not accept operator input".into(),
                ));
            }
        }
        self.append_run_event(
            run_id,
            "workflow.input.provided.v1",
            json!({"step_id": step_id(step), "input": input.clone()}),
        )?;
        self.append_run_event(
            run_id,
            "workflow.step.completed.v1",
            json!({
                "root_index": root_index,
                "step_id": step_id(step),
                "output": input,
            }),
        )?;
        self.resume_run(run_id).await
    }

    /// Resume a waiting or interrupted run without silently retrying completed steps.
    pub async fn resume_run(&self, run_id: &str) -> Result<WorkflowRun, WorkflowError> {
        let run = self.get_run(run_id)?;
        if !matches!(
            run.status,
            WorkflowStatus::Waiting | WorkflowStatus::Interrupted
        ) {
            return Err(WorkflowError::InvalidTransition(format!(
                "run {run_id} is not resumable"
            )));
        }
        let (definition, current_hash) = self
            .repository
            .definition(&run.workflow_name, &run.workflow_version)?
            .ok_or_else(|| WorkflowError::NotFound(run.workflow_name.clone()))?;
        if current_hash != run.workflow_hash {
            return Err(WorkflowError::InvalidTransition(
                "workflow definition changed; pinned run trust is invalid".into(),
            ));
        }
        if run.status == WorkflowStatus::Interrupted {
            let root_index = usize::try_from(run.completed_steps)
                .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?;
            if let Some(step) = definition.steps.get(root_index)
                && matches!(
                    step,
                    WorkflowStep::Agent {
                        idempotency: None,
                        ..
                    } | WorkflowStep::Tool {
                        idempotency: None,
                        ..
                    }
                )
            {
                return Err(WorkflowError::InvalidTransition(
                    "unknown non-idempotent effect cannot be retried by resume".into(),
                ));
            }
        }
        self.append_run_event(
            run_id,
            "workflow.run.resumed.v1",
            json!({"from_status": run.status}),
        )?;
        self.drive(
            run_id,
            definition,
            current_hash,
            run.inputs,
            run.completed_steps,
        )
        .await?;
        self.get_run(run_id)
    }

    /// Cancel a non-terminal run. Compensation, if configured later, is separate.
    pub fn cancel_run(&self, run_id: &str) -> Result<WorkflowRun, WorkflowError> {
        let run = self.get_run(run_id)?;
        if matches!(
            run.status,
            WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
        ) {
            return Err(WorkflowError::InvalidTransition(format!(
                "run {run_id} is terminal"
            )));
        }
        self.append_run_event(
            run_id,
            "workflow.run.cancelled.v1",
            json!({"reason": "operator requested cancellation"}),
        )?;
        self.get_run(run_id)
    }

    /// Mark abandoned running runs interrupted during startup recovery.
    pub fn recover_interrupted(&self) -> Result<Vec<WorkflowRun>, WorkflowError> {
        let running = self
            .list_runs(usize::MAX)?
            .into_iter()
            .filter(|run| run.status == WorkflowStatus::Running)
            .collect::<Vec<_>>();
        let mut recovered = Vec::with_capacity(running.len());
        for run in running {
            self.append_run_event(
                &run.run_id,
                "workflow.run.interrupted.v1",
                json!({"reason": "startup found an abandoned running attempt"}),
            )?;
            recovered.push(self.get_run(&run.run_id)?);
        }
        Ok(recovered)
    }

    /// Drain work that is safe to run automatically.
    ///
    /// The alpha has no queued trigger source yet, so this deliberately never resumes
    /// waiting or interrupted work and therefore never retries an uncertain effect.
    pub async fn drain(&self) -> Result<Vec<WorkflowRun>, WorkflowError> {
        Ok(Vec::new())
    }

    fn run_version(&self, run_id: &str) -> Result<u64, StoreError> {
        u64::try_from(
            self.journal
                .read_stream(&format!("workflow-run:{run_id}"))?
                .len(),
        )
        .map_err(|error| StoreError::Adapter(error.to_string()))
    }

    fn append_run_event(
        &self,
        run_id: &str,
        event_type: &str,
        payload: Value,
    ) -> Result<(), StoreError> {
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let expected_version = self.run_version(run_id)?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: format!("workflow-run:{run_id}"),
            expected_stream_version: expected_version,
            classification: EventClassification::Workflow,
            event_type: event_type.into(),
            actor: Actor {
                actor_type: ActorType::Workflow,
                id: run_id.into(),
            },
            context: ExecutionContext {
                correlation_id: run_id.into(),
                run_id: Some(run_id.into()),
                workflow_id: Some(run_id.into()),
                ..ExecutionContext::default()
            },
            payload,
        })?;
        Ok(())
    }

    async fn drive(
        &self,
        run_id: &str,
        definition: WorkflowDefinition,
        workflow_hash: String,
        inputs: Value,
        start_index: u32,
    ) -> Result<(), WorkflowError> {
        let mut context = json!({"inputs": inputs, "steps": {}});
        let budget = Arc::new(AtomicU32::new(0));
        let semaphore = Arc::new(Semaphore::new(
            usize::try_from(definition.max_concurrency)
                .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?,
        ));
        for (index, step) in definition.steps.iter().enumerate().skip(
            usize::try_from(start_index)
                .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?,
        ) {
            match self
                .execute_step(
                    run_id,
                    &workflow_hash,
                    step,
                    &mut context,
                    Arc::clone(&budget),
                    definition.step_budget,
                    Arc::clone(&semaphore),
                )
                .await
            {
                Ok(StepState::Completed(output)) => {
                    context["steps"][step_id(step)] = output.clone();
                    self.append_run_event(
                        run_id,
                        "workflow.step.completed.v1",
                        json!({
                            "root_index": index,
                            "step_id": step_id(step),
                            "output": output,
                        }),
                    )?;
                }
                Ok(StepState::Waiting(reason)) => {
                    self.append_run_event(
                        run_id,
                        "workflow.run.waiting.v1",
                        json!({"step_id": step_id(step), "reason": reason}),
                    )?;
                    return Ok(());
                }
                Err(WorkflowError::OutcomeUnknown(message)) => {
                    self.append_run_event(
                        run_id,
                        "workflow.run.interrupted.v1",
                        json!({"step_id": step_id(step), "reason": message}),
                    )?;
                    return Ok(());
                }
                Err(error) => {
                    self.append_run_event(
                        run_id,
                        "workflow.run.failed.v1",
                        json!({"step_id": step_id(step), "reason": error.to_string()}),
                    )?;
                    return Ok(());
                }
            }
        }
        let outputs = context.get("steps").cloned().unwrap_or(Value::Null);
        if let Err(error) = validate_instance(&definition.outputs, &outputs, "output") {
            self.append_run_event(
                run_id,
                "workflow.run.failed.v1",
                json!({"reason": error.to_string(), "phase": "output_validation"}),
            )?;
            return Ok(());
        }
        self.append_run_event(
            run_id,
            "workflow.run.completed.v1",
            json!({"outputs": outputs}),
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    #[async_recursion]
    async fn execute_step(
        &self,
        run_id: &str,
        workflow_hash: &str,
        step: &WorkflowStep,
        context: &mut Value,
        budget: Arc<AtomicU32>,
        step_budget: u32,
        semaphore: Arc<Semaphore>,
    ) -> Result<StepState, WorkflowError> {
        let attempt = budget.fetch_add(1, Ordering::AcqRel).saturating_add(1);
        if attempt > step_budget {
            return Err(WorkflowError::InvalidTransition(
                "total step-attempt budget exhausted".into(),
            ));
        }
        self.append_run_event(
            run_id,
            "workflow.step.started.v1",
            json!({"step_id": step_id(step), "attempt": attempt}),
        )?;
        match step {
            WorkflowStep::Emit { value, .. } => Ok(StepState::Completed(value.clone())),
            WorkflowStep::WaitForInput { prompt, .. } => Ok(StepState::Waiting(prompt.clone())),
            WorkflowStep::Approval { prompt, .. } => Ok(StepState::Waiting(prompt.clone())),
            WorkflowStep::Agent {
                id,
                prompt,
                idempotency,
            } => {
                let _permit = semaphore
                    .acquire()
                    .await
                    .map_err(|error| WorkflowError::Effect(error.to_string()))?;
                self.effects
                    .run(WorkflowEffect {
                        kind: "agent".into(),
                        action: "agent.run".into(),
                        content: json!({"prompt": prompt}),
                        idempotency: idempotency.clone(),
                        run_id: run_id.into(),
                        step_id: id.clone(),
                        workflow_hash: workflow_hash.into(),
                        attempt,
                    })
                    .await
                    .map(StepState::Completed)
            }
            WorkflowStep::Tool {
                id,
                tool,
                arguments,
                idempotency,
            } => {
                let _permit = semaphore
                    .acquire()
                    .await
                    .map_err(|error| WorkflowError::Effect(error.to_string()))?;
                self.effects
                    .run(WorkflowEffect {
                        kind: "tool".into(),
                        action: tool.clone(),
                        content: arguments.clone(),
                        idempotency: idempotency.clone(),
                        run_id: run_id.into(),
                        step_id: id.clone(),
                        workflow_hash: workflow_hash.into(),
                        attempt,
                    })
                    .await
                    .map(StepState::Completed)
            }
            WorkflowStep::Workflow {
                id,
                workflow,
                version,
                inputs,
            } => self
                .effects
                .run(WorkflowEffect {
                    kind: "workflow".into(),
                    action: "workflow.start".into(),
                    content: json!({
                        "workflow": workflow,
                        "version": version,
                        "inputs": inputs,
                    }),
                    idempotency: Some(format!("subworkflow:{run_id}:{id}")),
                    run_id: run_id.into(),
                    step_id: id.clone(),
                    workflow_hash: workflow_hash.into(),
                    attempt,
                })
                .await
                .map(StepState::Completed),
            WorkflowStep::Condition {
                expression,
                then,
                otherwise,
                ..
            } => {
                let condition = Condition::parse(expression)?;
                let selected = if condition.evaluate(context) {
                    then
                } else {
                    otherwise
                };
                self.execute_sequence(
                    run_id,
                    workflow_hash,
                    selected,
                    context,
                    budget,
                    step_budget,
                    semaphore,
                )
                .await
            }
            WorkflowStep::Parallel {
                branches,
                max_concurrency,
                ..
            } => {
                self.execute_parallel(
                    run_id,
                    workflow_hash,
                    branches,
                    *max_concurrency,
                    context,
                    budget,
                    step_budget,
                    semaphore,
                )
                .await
            }
            WorkflowStep::Foreach {
                items,
                max_items,
                steps,
                ..
            } => {
                let values = context
                    .pointer(items)
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(|| {
                        WorkflowError::InvalidTransition(format!(
                            "foreach pointer {items} is not an array"
                        ))
                    })?;
                if values.len()
                    > usize::try_from(*max_items)
                        .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?
                {
                    return Err(WorkflowError::InvalidTransition(
                        "foreach input exceeds declared maximum".into(),
                    ));
                }
                let mut outputs = Vec::with_capacity(values.len());
                for (index, item) in values.into_iter().enumerate() {
                    let mut iteration = context.clone();
                    iteration["item"] = item;
                    iteration["index"] = json!(index);
                    let state = self
                        .execute_sequence(
                            run_id,
                            workflow_hash,
                            steps,
                            &mut iteration,
                            Arc::clone(&budget),
                            step_budget,
                            Arc::clone(&semaphore),
                        )
                        .await?;
                    if let StepState::Waiting(reason) = state {
                        return Ok(StepState::Waiting(reason));
                    }
                    outputs.push(iteration);
                }
                Ok(StepState::Completed(Value::Array(outputs)))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_parallel(
        &self,
        run_id: &str,
        workflow_hash: &str,
        branches: &[Vec<WorkflowStep>],
        max_concurrency: u32,
        context: &Value,
        budget: Arc<AtomicU32>,
        step_budget: u32,
        semaphore: Arc<Semaphore>,
    ) -> Result<StepState, WorkflowError> {
        let concurrency = usize::try_from(max_concurrency)
            .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?;
        let base_context = context.clone();
        let owned_branches = branches.to_vec();
        let results = stream::iter(owned_branches.into_iter().enumerate())
            .map(|(index, branch)| {
                let mut branch_context = base_context.clone();
                let budget = Arc::clone(&budget);
                let semaphore = Arc::clone(&semaphore);
                async move {
                    let state = self
                        .execute_sequence(
                            run_id,
                            workflow_hash,
                            &branch,
                            &mut branch_context,
                            budget,
                            step_budget,
                            semaphore,
                        )
                        .await?;
                    Ok::<_, WorkflowError>((index, state, branch_context))
                }
            })
            .buffer_unordered(concurrency)
            .try_collect::<Vec<_>>()
            .await?;
        let mut ordered = results;
        ordered.sort_by_key(|(index, _, _)| *index);
        if ordered
            .iter()
            .any(|(_, state, _)| matches!(state, StepState::Waiting(_)))
        {
            return Ok(StepState::Waiting("parallel branch is waiting".into()));
        }
        Ok(StepState::Completed(Value::Array(
            ordered
                .into_iter()
                .map(|(_, _, branch_context)| branch_context)
                .collect(),
        )))
    }

    #[allow(clippy::too_many_arguments)]
    #[async_recursion]
    async fn execute_sequence(
        &self,
        run_id: &str,
        workflow_hash: &str,
        steps: &[WorkflowStep],
        context: &mut Value,
        budget: Arc<AtomicU32>,
        step_budget: u32,
        semaphore: Arc<Semaphore>,
    ) -> Result<StepState, WorkflowError> {
        for step in steps {
            match self
                .execute_step(
                    run_id,
                    workflow_hash,
                    step,
                    context,
                    Arc::clone(&budget),
                    step_budget,
                    Arc::clone(&semaphore),
                )
                .await?
            {
                StepState::Completed(output) => {
                    context["steps"][step_id(step)] = output;
                }
                waiting @ StepState::Waiting(_) => return Ok(waiting),
            }
        }
        Ok(StepState::Completed(
            context.get("steps").cloned().unwrap_or(Value::Null),
        ))
    }
}

enum StepState {
    Completed(Value),
    Waiting(String),
}

fn validate_instance(schema: &Value, instance: &Value, label: &str) -> Result<(), WorkflowError> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| WorkflowError::Schema(error.to_string()))?;
    let errors = validator
        .iter_errors(instance)
        .take(8)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(WorkflowError::Schema(format!(
            "{label}: {}",
            errors.join("; ")
        )))
    }
}

/// Effect runner for validation-only/offline workflows containing only pure steps.
pub struct DenyWorkflowEffects;

#[async_trait]
impl WorkflowEffectRunner for DenyWorkflowEffects {
    async fn run(&self, effect: WorkflowEffect) -> Result<Value, WorkflowError> {
        Err(WorkflowError::Effect(format!(
            "no runtime adapter is configured for {} step {}",
            effect.kind, effect.step_id
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Condition, DenyWorkflowEffects, EventSourcedWorkflowRepository, WorkflowService,
        validate_definition,
    };
    use colossus_ports::{EventJournal, WorkflowRepository};
    use colossus_testkit::InMemoryEventJournal;
    use serde_json::json;
    use std::sync::Arc;

    const SIMPLE: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: smoke
  version: 1.0.0
  description: Offline smoke workflow
inputs:
  type: object
  required: [message]
  properties:
    message: { type: string }
outputs:
  type: object
capabilities: []
maxConcurrency: 2
stepBudget: 4
steps:
  - type: emit
    id: result
    value: { ok: true }
"#;

    const WAITING: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: waiting
  version: 1.0.0
  description: Input wait workflow
inputs: { type: object }
outputs: { type: object }
capabilities: []
maxConcurrency: 2
stepBudget: 4
steps:
  - type: wait_for_input
    id: answer
    prompt: Supply an answer
    schema: { type: string }
  - type: emit
    id: done
    value: { ok: true }
"#;

    const PARALLEL: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: parallel
  version: 1.0.0
  description: Bounded parallel workflow
inputs: { type: object }
outputs: { type: object }
capabilities: []
maxConcurrency: 2
stepBudget: 4
steps:
  - type: parallel
    id: branches
    max_concurrency: 2
    branches:
      - [{ type: emit, id: left, value: 1 }]
      - [{ type: emit, id: right, value: 2 }]
"#;

    #[test]
    fn strict_yaml_hashes_exact_content_and_rejects_code_fields() {
        let validated = validate_definition(SIMPLE).expect("valid");
        let with_space = validate_definition(&format!("{SIMPLE}\n")).expect("valid with space");
        assert_ne!(validated.content_hash, with_space.content_hash);
        let executable = SIMPLE.replace("value: { ok: true }", "shell: whoami");
        assert!(validate_definition(&executable).is_err());
    }

    #[test]
    fn condition_grammar_is_non_executable_and_evaluates_json_pointers() {
        let condition =
            Condition::parse("exists(/inputs/name) && /inputs/count >= 2").expect("condition");
        assert!(condition.evaluate(&json!({"inputs":{"name":"a","count":2}})));
        assert!(Condition::parse("system(\"whoami\")").is_err());
    }

    #[tokio::test]
    async fn event_sourced_run_completes_and_definition_change_invalidates_trust() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let service = WorkflowService::new(
            Arc::clone(&journal),
            Arc::clone(&repository),
            Arc::new(DenyWorkflowEffects),
        );
        let registered = service
            .register_definition(SIMPLE, "repo:.colossus/workflows/smoke.yaml")
            .expect("register");
        let run = service
            .start_run("smoke", "1.0.0", json!({"message":"hello"}))
            .await
            .expect("run");
        assert_eq!(run.status, colossus_contracts::WorkflowStatus::Completed);
        assert_eq!(run.workflow_hash, registered.content_hash);

        service
            .register_definition(
                &SIMPLE.replace("Offline smoke workflow", "Changed workflow"),
                "repo:.colossus/workflows/smoke.yaml",
            )
            .expect("changed definition");
        let events = journal
            .read_stream("workflow-definition:smoke:1.0.0")
            .expect("events");
        assert_eq!(
            events.last().expect("last").event_type,
            "workflow.definition.changed.v1"
        );
    }

    #[tokio::test]
    async fn input_completes_waiting_step_before_resume() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let service = WorkflowService::new(journal, repository, Arc::new(DenyWorkflowEffects));
        service
            .register_definition(WAITING, "test")
            .expect("register");
        let waiting = service
            .start_run("waiting", "1.0.0", json!({}))
            .await
            .expect("start");
        assert_eq!(waiting.status, colossus_contracts::WorkflowStatus::Waiting);
        let completed = service
            .provide_input(&waiting.run_id, json!("accepted"))
            .await
            .expect("input");
        assert_eq!(
            completed.status,
            colossus_contracts::WorkflowStatus::Completed
        );
        assert_eq!(completed.completed_steps, 2);
    }

    #[tokio::test]
    async fn parallel_step_serializes_durable_events_without_losing_concurrency_bounds() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let service = WorkflowService::new(journal, repository, Arc::new(DenyWorkflowEffects));
        service
            .register_definition(PARALLEL, "test")
            .expect("register");
        let run = service
            .start_run("parallel", "1.0.0", json!({}))
            .await
            .expect("parallel run");
        assert_eq!(run.status, colossus_contracts::WorkflowStatus::Completed);
    }

    #[test]
    fn direct_recursive_workflow_is_rejected() {
        let recursive = SIMPLE.replace(
            "- type: emit\n    id: result\n    value: { ok: true }",
            "- type: workflow\n    id: recurse\n    workflow: smoke\n    version: 1.0.0\n    inputs: {}",
        );
        assert!(validate_definition(&recursive).is_err());
    }
}
