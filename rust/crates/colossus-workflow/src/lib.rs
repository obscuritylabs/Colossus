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
const MAX_WORKFLOW_CALL_DEPTH: usize = 16;

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
    let mut step_count = validate_steps(&definition.steps, definition.max_concurrency, &mut ids)?;
    step_count = step_count.saturating_add(validate_compensation_steps(
        &definition.compensation,
        &mut ids,
    )?);
    reject_direct_workflow_cycle(
        &definition.steps,
        &definition.metadata.name,
        &definition.metadata.version,
    )?;
    reject_direct_workflow_cycle(
        &definition.compensation,
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

fn validate_compensation_steps(
    steps: &[WorkflowStep],
    ids: &mut BTreeSet<String>,
) -> Result<u32, WorkflowError> {
    for step in steps {
        let id = step_id(step);
        if !valid_step_id(id) || !ids.insert(id.to_owned()) {
            return Err(WorkflowError::InvalidDefinition(format!(
                "compensation step id {id:?} is invalid or duplicated"
            )));
        }
        match step {
            WorkflowStep::Agent {
                idempotency: Some(strategy),
                ..
            }
            | WorkflowStep::Tool {
                idempotency: Some(strategy),
                ..
            } if !strategy.is_empty() => {}
            WorkflowStep::Agent { .. } | WorkflowStep::Tool { .. } => {
                return Err(WorkflowError::InvalidDefinition(
                    "compensation effects require an explicit idempotency strategy".into(),
                ));
            }
            _ => {
                return Err(WorkflowError::InvalidDefinition(
                    "compensation supports only idempotent agent or tool steps".into(),
                ));
            }
        }
    }
    u32::try_from(steps.len()).map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))
}

fn workflow_references(steps: &[WorkflowStep], references: &mut Vec<(String, String)>) {
    for step in steps {
        match step {
            WorkflowStep::Workflow {
                workflow, version, ..
            } => references.push((workflow.clone(), version.clone())),
            WorkflowStep::Condition {
                then, otherwise, ..
            } => {
                workflow_references(then, references);
                workflow_references(otherwise, references);
            }
            WorkflowStep::Parallel { branches, .. } => {
                for branch in branches {
                    workflow_references(branch, references);
                }
            }
            WorkflowStep::Foreach { steps, .. } => workflow_references(steps, references),
            _ => {}
        }
    }
}

fn validate_call_graph(
    repository: &dyn WorkflowRepository,
    proposed: &WorkflowDefinition,
    require_complete: bool,
) -> Result<(), WorkflowError> {
    fn visit(
        repository: &dyn WorkflowRepository,
        proposed: &WorkflowDefinition,
        name: &str,
        version: &str,
        require_complete: bool,
        stack: &mut Vec<(String, String)>,
    ) -> Result<(), WorkflowError> {
        let key = (name.to_owned(), version.to_owned());
        if let Some(position) = stack.iter().position(|entry| entry == &key) {
            let mut cycle = stack[position..]
                .iter()
                .map(|(name, version)| format!("{name}:{version}"))
                .collect::<Vec<_>>();
            cycle.push(format!("{name}:{version}"));
            return Err(WorkflowError::InvalidDefinition(format!(
                "workflow call cycle detected: {}",
                cycle.join(" -> ")
            )));
        }
        if stack.len() >= MAX_WORKFLOW_CALL_DEPTH {
            return Err(WorkflowError::InvalidDefinition(format!(
                "workflow call depth exceeds {MAX_WORKFLOW_CALL_DEPTH}"
            )));
        }
        let definition = if name == proposed.metadata.name && version == proposed.metadata.version {
            proposed.clone()
        } else {
            match repository.definition(name, version)? {
                Some((definition, _)) => definition,
                None if require_complete => {
                    return Err(WorkflowError::NotFound(format!("{name}:{version}")));
                }
                None => return Ok(()),
            }
        };
        stack.push(key);
        let mut references = Vec::new();
        workflow_references(&definition.steps, &mut references);
        for (child_name, child_version) in references {
            visit(
                repository,
                proposed,
                &child_name,
                &child_version,
                require_complete,
                stack,
            )?;
        }
        stack.pop();
        Ok(())
    }

    visit(
        repository,
        proposed,
        &proposed.metadata.name,
        &proposed.metadata.version,
        require_complete,
        &mut Vec::new(),
    )
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

fn find_step<'a>(steps: &'a [WorkflowStep], id: &str) -> Option<&'a WorkflowStep> {
    for step in steps {
        if step_id(step) == id {
            return Some(step);
        }
        let found = match step {
            WorkflowStep::Condition {
                then, otherwise, ..
            } => find_step(then, id).or_else(|| find_step(otherwise, id)),
            WorkflowStep::Parallel { branches, .. } => {
                branches.iter().find_map(|branch| find_step(branch, id))
            }
            WorkflowStep::Foreach { steps, .. } => find_step(steps, id),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

fn step_retryable(step: &WorkflowStep) -> bool {
    matches!(
        step,
        WorkflowStep::Agent {
            idempotency: Some(_),
            ..
        } | WorkflowStep::Tool {
            idempotency: Some(_),
            ..
        } | WorkflowStep::Workflow { .. }
    )
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
        let mut seen = BTreeSet::new();
        for event in events {
            if matches!(
                event.event_type.as_str(),
                "workflow.run.queued.v1" | "workflow.run.started.v1"
            ) && let Some(run_id) = event.stream_id.strip_prefix("workflow-run:")
                && seen.insert(run_id.to_owned())
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
        status: if first.event_type == "workflow.run.queued.v1" {
            WorkflowStatus::Queued
        } else {
            WorkflowStatus::Running
        },
        inputs: start.get("inputs").cloned().unwrap_or(Value::Null),
        outputs: None,
        completed_steps: 0,
    };
    for event in events.iter().skip(1) {
        let payload = journal.decrypt_payload(event)?;
        match event.event_type.as_str() {
            "workflow.run.queued.v1" => run.status = WorkflowStatus::Queued,
            "workflow.run.started.v1" => run.status = WorkflowStatus::Running,
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
    /// Whether this is an explicit compensation effect.
    pub compensation: bool,
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
        validate_call_graph(self.repository.as_ref(), &validated.definition, false)?;
        self.repository
            .register(&validated.definition, &validated.content_hash, provenance)?;
        Ok(validated)
    }

    /// Queue a validated, hash-pinned run for a worker or embedded caller.
    pub fn queue_run(
        &self,
        name: &str,
        version: &str,
        inputs: Value,
    ) -> Result<WorkflowRun, WorkflowError> {
        let (definition, workflow_hash) = self
            .repository
            .definition(name, version)?
            .ok_or_else(|| WorkflowError::NotFound(format!("{name}:{version}")))?;
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        validate_instance(&definition.inputs, &inputs, "input")?;
        let run_id = Uuid::now_v7().to_string();
        self.append_run_event(
            &run_id,
            "workflow.run.queued.v1",
            json!({
                "workflow_name": name,
                "workflow_version": version,
                "workflow_hash": workflow_hash,
                "inputs": inputs,
            }),
        )?;
        self.get_run(&run_id)
    }

    /// Start and drive a run until it waits or reaches a terminal state.
    pub async fn start_run(
        &self,
        name: &str,
        version: &str,
        inputs: Value,
    ) -> Result<WorkflowRun, WorkflowError> {
        let queued = self.queue_run(name, version, inputs)?;
        self.run_queued(&queued.run_id).await
    }

    async fn run_queued(&self, run_id: &str) -> Result<WorkflowRun, WorkflowError> {
        let run = self.get_run(run_id)?;
        if run.status != WorkflowStatus::Queued {
            return Err(WorkflowError::InvalidTransition(format!(
                "run {run_id} is not queued"
            )));
        }
        let (definition, current_hash) = self
            .repository
            .definition(&run.workflow_name, &run.workflow_version)?
            .ok_or_else(|| WorkflowError::NotFound(run.workflow_name.clone()))?;
        if current_hash != run.workflow_hash {
            return Err(WorkflowError::InvalidTransition(
                "workflow definition changed; queued run trust is invalid".into(),
            ));
        }
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        self.append_run_event(
            run_id,
            "workflow.run.started.v1",
            json!({"from_status": "queued"}),
        )?;
        self.drive(run_id, definition, current_hash, run.inputs, 0)
            .await?;
        self.get_run(run_id)
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
        let waiting_step_id = self
            .journal
            .read_stream(&format!("workflow-run:{run_id}"))?
            .iter()
            .rev()
            .find(|event| event.event_type == "workflow.run.waiting.v1")
            .map(|event| self.journal.decrypt_payload(event))
            .transpose()?
            .and_then(|payload| {
                payload
                    .get("step_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .ok_or_else(|| {
                WorkflowError::InvalidTransition(
                    "waiting step id is absent from the journal".into(),
                )
            })?;
        let step = find_step(&definition.steps, &waiting_step_id).ok_or_else(|| {
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
        let is_root = definition
            .steps
            .get(root_index)
            .is_some_and(|root| step_id(root) == step_id(step));
        let mut completion = json!({
            "step_id": step_id(step),
            "output": input,
        });
        if is_root {
            completion["root_index"] = json!(root_index);
        }
        self.append_run_event(run_id, "workflow.step.completed.v1", completion)?;
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
            let events = self
                .journal
                .read_stream(&format!("workflow-run:{run_id}"))?;
            let uncertain_retryable = events
                .iter()
                .rev()
                .find(|event| event.event_type == "workflow.step.outcome_unknown.v1")
                .map(|event| self.journal.decrypt_payload(event))
                .transpose()?
                .and_then(|payload| payload.get("retry_allowed").and_then(Value::as_bool));
            if uncertain_retryable == Some(false) {
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
            let events = self
                .journal
                .read_stream(&format!("workflow-run:{}", run.run_id))?;
            let latest_started = events.iter().enumerate().rev().find(|(_, event)| {
                matches!(
                    event.event_type.as_str(),
                    "workflow.step.started.v1" | "workflow.compensation.step.started.v1"
                )
            });
            if let Some((started_index, started_event)) = latest_started {
                let started = self.journal.decrypt_payload(started_event)?;
                let step_id = started
                    .get("step_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let completed_after = events[started_index.saturating_add(1)..]
                    .iter()
                    .filter(|event| {
                        matches!(
                            event.event_type.as_str(),
                            "workflow.step.completed.v1"
                                | "workflow.compensation.step.completed.v1"
                        )
                    })
                    .map(|event| self.journal.decrypt_payload(event))
                    .collect::<Result<Vec<_>, _>>()?
                    .iter()
                    .any(|payload| payload.get("step_id").and_then(Value::as_str) == Some(step_id));
                if completed_after {
                    self.append_run_event(
                        &run.run_id,
                        "workflow.run.interrupted.v1",
                        json!({"reason": "startup found an abandoned run after a completed step"}),
                    )?;
                    recovered.push(self.get_run(&run.run_id)?);
                    continue;
                }
                let retry_allowed = self
                    .repository
                    .definition(&run.workflow_name, &run.workflow_version)?
                    .and_then(|(definition, _)| {
                        find_step(&definition.steps, step_id)
                            .map(step_retryable)
                            .or_else(|| {
                                find_step(&definition.compensation, step_id).map(step_retryable)
                            })
                    })
                    .unwrap_or(false);
                self.append_run_event(
                    &run.run_id,
                    "workflow.step.outcome_unknown.v1",
                    json!({
                        "step_id": step_id,
                        "attempt": started.get("attempt").cloned().unwrap_or(Value::Null),
                        "retry_allowed": retry_allowed,
                        "reason": "startup found an abandoned step attempt",
                    }),
                )?;
            }
            self.append_run_event(
                &run.run_id,
                "workflow.run.interrupted.v1",
                json!({"reason": "startup found an abandoned running attempt"}),
            )?;
            recovered.push(self.get_run(&run.run_id)?);
        }
        Ok(recovered)
    }

    /// Drain queued work without resuming waiting or interrupted attempts.
    pub async fn drain(&self) -> Result<Vec<WorkflowRun>, WorkflowError> {
        let queued = self
            .list_runs(usize::MAX)?
            .into_iter()
            .filter(|run| run.status == WorkflowStatus::Queued)
            .collect::<Vec<_>>();
        let mut completed = Vec::with_capacity(queued.len());
        for run in queued {
            completed.push(self.run_queued(&run.run_id).await?);
        }
        Ok(completed)
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
        let events = self
            .journal
            .read_stream(&format!("workflow-run:{run_id}"))?;
        let mut context = json!({"inputs": inputs, "steps": {}});
        for event in &events {
            if event.event_type == "workflow.step.completed.v1" {
                let payload = self.journal.decrypt_payload(event)?;
                if let (Some(step_id), Some(output)) = (
                    payload.get("step_id").and_then(Value::as_str),
                    payload.get("output").cloned(),
                ) {
                    context["steps"][step_id] = output;
                }
            }
        }
        let attempts = events
            .iter()
            .filter(|event| {
                matches!(
                    event.event_type.as_str(),
                    "workflow.step.started.v1" | "workflow.compensation.step.started.v1"
                )
            })
            .count();
        let budget = Arc::new(AtomicU32::new(
            u32::try_from(attempts)
                .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?,
        ));
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
                Ok(StepState::Waiting {
                    step_id: waiting_step_id,
                    reason,
                }) => {
                    self.append_run_event(
                        run_id,
                        "workflow.run.waiting.v1",
                        json!({"step_id": waiting_step_id, "reason": reason}),
                    )?;
                    return Ok(());
                }
                Err(WorkflowError::OutcomeUnknown(message)) => {
                    self.append_run_event(
                        run_id,
                        "workflow.step.outcome_unknown.v1",
                        json!({
                            "step_id": step_id(step),
                            "retry_allowed": step_retryable(step),
                            "reason": &message,
                        }),
                    )?;
                    self.append_run_event(
                        run_id,
                        "workflow.run.interrupted.v1",
                        json!({"step_id": step_id(step), "reason": message}),
                    )?;
                    return Ok(());
                }
                Err(error) => {
                    let compensation = self
                        .run_compensation(
                            run_id,
                            &workflow_hash,
                            &definition.compensation,
                            Arc::clone(&budget),
                            definition.step_budget,
                            Arc::clone(&semaphore),
                        )
                        .await;
                    if let Err(WorkflowError::OutcomeUnknown(message)) = &compensation {
                        self.append_run_event(
                            run_id,
                            "workflow.step.outcome_unknown.v1",
                            json!({
                                "phase": "compensation",
                                "retry_allowed": false,
                                "reason": message,
                            }),
                        )?;
                        self.append_run_event(
                            run_id,
                            "workflow.run.interrupted.v1",
                            json!({"phase": "compensation", "reason": message}),
                        )?;
                        return Ok(());
                    }
                    self.append_run_event(
                        run_id,
                        "workflow.run.failed.v1",
                        json!({
                            "step_id": step_id(step),
                            "reason": error.to_string(),
                            "compensation": compensation.err().map(|error| error.to_string()),
                        }),
                    )?;
                    return Ok(());
                }
            }
        }
        let outputs = context.get("steps").cloned().unwrap_or(Value::Null);
        if let Err(error) = validate_instance(&definition.outputs, &outputs, "output") {
            let compensation = self
                .run_compensation(
                    run_id,
                    &workflow_hash,
                    &definition.compensation,
                    Arc::clone(&budget),
                    definition.step_budget,
                    Arc::clone(&semaphore),
                )
                .await;
            if let Err(WorkflowError::OutcomeUnknown(message)) = &compensation {
                self.append_run_event(
                    run_id,
                    "workflow.step.outcome_unknown.v1",
                    json!({
                        "phase": "compensation",
                        "retry_allowed": false,
                        "reason": message,
                    }),
                )?;
                self.append_run_event(
                    run_id,
                    "workflow.run.interrupted.v1",
                    json!({"phase": "compensation", "reason": message}),
                )?;
                return Ok(());
            }
            self.append_run_event(
                run_id,
                "workflow.run.failed.v1",
                json!({
                    "reason": error.to_string(),
                    "phase": "output_validation",
                    "compensation": compensation.err().map(|error| error.to_string()),
                }),
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
        if let Some(output) = context
            .get("steps")
            .and_then(|steps| steps.get(step_id(step)))
            .cloned()
        {
            return Ok(StepState::Completed(output));
        }
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
            WorkflowStep::WaitForInput { id, prompt, .. } => Ok(StepState::Waiting {
                step_id: id.clone(),
                reason: prompt.clone(),
            }),
            WorkflowStep::Approval { id, prompt, .. } => Ok(StepState::Waiting {
                step_id: id.clone(),
                reason: prompt.clone(),
            }),
            WorkflowStep::Agent {
                id,
                prompt,
                idempotency,
            } => {
                let _permit = semaphore
                    .acquire()
                    .await
                    .map_err(|error| WorkflowError::Effect(error.to_string()))?;
                self.run_effect_with_retry(
                    WorkflowEffect {
                        kind: "agent".into(),
                        action: "agent.run".into(),
                        content: json!({"prompt": prompt}),
                        idempotency: idempotency.clone(),
                        run_id: run_id.into(),
                        step_id: id.clone(),
                        workflow_hash: workflow_hash.into(),
                        attempt,
                        compensation: false,
                    },
                    Arc::clone(&budget),
                    step_budget,
                )
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
                self.run_effect_with_retry(
                    WorkflowEffect {
                        kind: "tool".into(),
                        action: tool.clone(),
                        content: arguments.clone(),
                        idempotency: idempotency.clone(),
                        run_id: run_id.into(),
                        step_id: id.clone(),
                        workflow_hash: workflow_hash.into(),
                        attempt,
                        compensation: false,
                    },
                    Arc::clone(&budget),
                    step_budget,
                )
                .await
                .map(StepState::Completed)
            }
            WorkflowStep::Workflow {
                id,
                workflow,
                version,
                inputs,
            } => self
                .run_effect_with_retry(
                    WorkflowEffect {
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
                        compensation: false,
                    },
                    Arc::clone(&budget),
                    step_budget,
                )
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
                    if let StepState::Waiting { step_id, reason } = state {
                        return Ok(StepState::Waiting { step_id, reason });
                    }
                    outputs.push(iteration);
                }
                Ok(StepState::Completed(Value::Array(outputs)))
            }
        }
    }

    async fn run_effect_with_retry(
        &self,
        mut effect: WorkflowEffect,
        budget: Arc<AtomicU32>,
        step_budget: u32,
    ) -> Result<Value, WorkflowError> {
        match self.effects.run(effect.clone()).await {
            Err(WorkflowError::Effect(first_error)) if effect.idempotency.is_some() => {
                let retry_attempt = budget.fetch_add(1, Ordering::AcqRel).saturating_add(1);
                if retry_attempt > step_budget {
                    return Err(WorkflowError::InvalidTransition(
                        "total step-attempt budget exhausted before idempotent retry".into(),
                    ));
                }
                self.append_run_event(
                    &effect.run_id,
                    "workflow.step.retrying.v1",
                    json!({
                        "step_id": effect.step_id,
                        "failed_attempt": effect.attempt,
                        "next_attempt": retry_attempt,
                        "reason": first_error,
                        "idempotency": effect.idempotency,
                    }),
                )?;
                effect.attempt = retry_attempt;
                self.append_run_event(
                    &effect.run_id,
                    "workflow.step.started.v1",
                    json!({"step_id": effect.step_id, "attempt": retry_attempt, "retry": true}),
                )?;
                self.effects.run(effect).await
            }
            result => result,
        }
    }

    async fn run_compensation(
        &self,
        run_id: &str,
        workflow_hash: &str,
        steps: &[WorkflowStep],
        budget: Arc<AtomicU32>,
        step_budget: u32,
        semaphore: Arc<Semaphore>,
    ) -> Result<(), WorkflowError> {
        for step in steps {
            let attempt = budget.fetch_add(1, Ordering::AcqRel).saturating_add(1);
            if attempt > step_budget {
                return Err(WorkflowError::InvalidTransition(
                    "total step-attempt budget exhausted during compensation".into(),
                ));
            }
            self.append_run_event(
                run_id,
                "workflow.compensation.step.started.v1",
                json!({"step_id": step_id(step), "attempt": attempt}),
            )?;
            let _permit = semaphore
                .acquire()
                .await
                .map_err(|error| WorkflowError::Effect(error.to_string()))?;
            let effect = match step {
                WorkflowStep::Agent {
                    id,
                    prompt,
                    idempotency,
                } => WorkflowEffect {
                    kind: "agent".into(),
                    action: "agent.run".into(),
                    content: json!({"prompt": prompt}),
                    idempotency: idempotency.clone(),
                    run_id: run_id.into(),
                    step_id: id.clone(),
                    workflow_hash: workflow_hash.into(),
                    attempt,
                    compensation: true,
                },
                WorkflowStep::Tool {
                    id,
                    tool,
                    arguments,
                    idempotency,
                } => WorkflowEffect {
                    kind: "tool".into(),
                    action: tool.clone(),
                    content: arguments.clone(),
                    idempotency: idempotency.clone(),
                    run_id: run_id.into(),
                    step_id: id.clone(),
                    workflow_hash: workflow_hash.into(),
                    attempt,
                    compensation: true,
                },
                _ => {
                    return Err(WorkflowError::InvalidDefinition(
                        "validated compensation contains an unsupported step".into(),
                    ));
                }
            };
            match self
                .run_effect_with_retry(effect, Arc::clone(&budget), step_budget)
                .await
            {
                Ok(output) => self.append_run_event(
                    run_id,
                    "workflow.compensation.step.completed.v1",
                    json!({"step_id": step_id(step), "output": output}),
                )?,
                Err(error) => {
                    self.append_run_event(
                        run_id,
                        "workflow.compensation.step.failed.v1",
                        json!({"step_id": step_id(step), "reason": error.to_string()}),
                    )?;
                    return Err(error);
                }
            }
        }
        Ok(())
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
        if let Some((step_id, reason)) = ordered.iter().find_map(|(_, state, _)| match state {
            StepState::Waiting { step_id, reason } => Some((step_id.clone(), reason.clone())),
            StepState::Completed(_) => None,
        }) {
            return Ok(StepState::Waiting { step_id, reason });
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
                    self.append_run_event(
                        run_id,
                        "workflow.step.completed.v1",
                        json!({"step_id": step_id(step), "output": context["steps"][step_id(step)]}),
                    )?;
                }
                waiting @ StepState::Waiting { .. } => return Ok(waiting),
            }
        }
        Ok(StepState::Completed(
            context.get("steps").cloned().unwrap_or(Value::Null),
        ))
    }
}

enum StepState {
    Completed(Value),
    Waiting { step_id: String, reason: String },
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
        Condition, DenyWorkflowEffects, EventSourcedWorkflowRepository, WorkflowEffect,
        WorkflowEffectRunner, WorkflowError, WorkflowService, validate_definition,
    };
    use async_trait::async_trait;
    use colossus_ports::{EventJournal, WorkflowRepository};
    use colossus_testkit::InMemoryEventJournal;
    use serde_json::json;
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

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

    #[derive(Default)]
    struct RecordingEffects {
        calls: Mutex<Vec<WorkflowEffect>>,
        failures: Mutex<BTreeMap<String, usize>>,
    }

    impl RecordingEffects {
        fn fail(&self, action: &str, times: usize) {
            self.failures
                .lock()
                .expect("failures")
                .insert(action.into(), times);
        }

        fn calls(&self) -> Vec<WorkflowEffect> {
            self.calls.lock().expect("calls").clone()
        }
    }

    #[async_trait]
    impl WorkflowEffectRunner for RecordingEffects {
        async fn run(&self, effect: WorkflowEffect) -> Result<serde_json::Value, WorkflowError> {
            self.calls.lock().expect("calls").push(effect.clone());
            let mut failures = self.failures.lock().expect("failures");
            if let Some(remaining) = failures.get_mut(&effect.action)
                && *remaining > 0
            {
                *remaining -= 1;
                return Err(WorkflowError::Effect(format!(
                    "injected failure for {}",
                    effect.action
                )));
            }
            Ok(json!({"action": effect.action, "compensation": effect.compensation}))
        }
    }

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
    async fn nested_input_wait_resumes_without_repeating_the_completed_wait() {
        const NESTED_WAIT: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: nested-wait
  version: 1.0.0
  description: Nested input wait
inputs:
  type: object
  required: [ask]
  properties: { ask: { type: boolean } }
outputs: { type: object }
capabilities: []
maxConcurrency: 1
stepBudget: 6
steps:
  - type: emit
    id: before
    value: { retained: true }
  - type: condition
    id: branch
    expression: /inputs/ask == true
    then:
      - type: wait_for_input
        id: nested-answer
        prompt: Supply nested input
        schema: { type: string }
      - type: emit
        id: nested-done
        value: { ok: true }
    otherwise:
      - type: emit
        id: skipped
        value: { ok: false }
"#;
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let service = WorkflowService::new(
            Arc::clone(&journal),
            repository,
            Arc::new(DenyWorkflowEffects),
        );
        service
            .register_definition(NESTED_WAIT, "test")
            .expect("register");
        let waiting = service
            .start_run("nested-wait", "1.0.0", json!({"ask": true}))
            .await
            .expect("start");
        assert_eq!(waiting.status, colossus_contracts::WorkflowStatus::Waiting);
        let completed = service
            .provide_input(&waiting.run_id, json!("accepted"))
            .await
            .expect("nested input");
        assert_eq!(
            completed.status,
            colossus_contracts::WorkflowStatus::Completed
        );
        let outputs = completed.outputs.expect("outputs");
        assert_eq!(outputs["before"], json!({"retained": true}));
        assert_eq!(outputs["nested-answer"], json!("accepted"));
        let events = journal
            .read_stream(&format!("workflow-run:{}", waiting.run_id))
            .expect("events");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "workflow.input.provided.v1")
                .count(),
            1
        );
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

    #[tokio::test]
    async fn queued_runs_are_claimed_only_by_start_or_drain() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let service = WorkflowService::new(journal, repository, Arc::new(DenyWorkflowEffects));
        service
            .register_definition(SIMPLE, "test")
            .expect("register");
        let queued = service
            .queue_run("smoke", "1.0.0", json!({"message":"queued"}))
            .expect("queue");
        assert_eq!(queued.status, colossus_contracts::WorkflowStatus::Queued);
        let drained = service.drain().await.expect("drain");
        assert_eq!(drained.len(), 1);
        assert_eq!(
            drained[0].status,
            colossus_contracts::WorkflowStatus::Completed
        );
        assert!(service.drain().await.expect("empty drain").is_empty());
    }

    #[test]
    fn indirect_cycles_and_excessive_call_depth_are_rejected() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let service = WorkflowService::new(journal, repository, Arc::new(DenyWorkflowEffects));
        let definition = |name: &str, target: &str| {
            format!(
                "apiVersion: colossus.dev/v1alpha1\nkind: Workflow\nmetadata:\n  name: {name}\n  version: 1.0.0\n  description: graph node\ninputs: {{ type: object }}\noutputs: {{ type: object }}\ncapabilities: []\nmaxConcurrency: 1\nstepBudget: 4\nsteps:\n  - type: workflow\n    id: call-{target}\n    workflow: {target}\n    version: 1.0.0\n    inputs: {{}}\n"
            )
        };
        service
            .register_definition(&definition("alpha", "beta"), "test")
            .expect("forward reference");
        let cycle = service
            .register_definition(&definition("beta", "alpha"), "test")
            .expect_err("indirect cycle");
        assert!(cycle.to_string().contains("cycle detected"));

        let leaf = SIMPLE
            .replace("name: smoke", "name: node16")
            .replace("Offline smoke workflow", "graph leaf");
        service.register_definition(&leaf, "test").expect("leaf");
        for index in (0..16).rev() {
            let name = format!("node{index}");
            let target = format!("node{}", index + 1);
            let result = service.register_definition(&definition(&name, &target), "test");
            if index == 0 {
                assert!(
                    result
                        .expect_err("depth limit")
                        .to_string()
                        .contains("call depth exceeds")
                );
            } else {
                result.expect("bounded graph node");
            }
        }
    }

    #[tokio::test]
    async fn idempotent_effects_retry_and_compensation_is_separately_dispatched() {
        const COMPENSATING: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: compensating
  version: 1.0.0
  description: Retry and compensate
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 8
steps:
  - type: tool
    id: primary
    tool: primary.fail
    arguments: { value: 1 }
    idempotency: primary-key
compensation:
  - type: tool
    id: rollback
    tool: rollback.run
    arguments: { value: 1 }
    idempotency: rollback-key
"#;
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let effects = Arc::new(RecordingEffects::default());
        effects.fail("primary.fail", 2);
        let service = WorkflowService::new(journal, repository, effects.clone());
        service
            .register_definition(COMPENSATING, "test")
            .expect("register");
        let run = service
            .start_run("compensating", "1.0.0", json!({}))
            .await
            .expect("run");
        assert_eq!(run.status, colossus_contracts::WorkflowStatus::Failed);
        let calls = effects.calls();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.action == "primary.fail")
                .count(),
            2
        );
        let rollback = calls
            .iter()
            .find(|call| call.action == "rollback.run")
            .expect("rollback call");
        assert!(rollback.compensation);
        assert_ne!(rollback.step_id, calls[0].step_id);
    }

    #[tokio::test]
    async fn crash_recovery_records_unknown_and_never_auto_retries() {
        const NON_IDEMPOTENT: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: crashy
  version: 1.0.0
  description: Crash recovery
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 2
steps:
  - type: tool
    id: mutate
    tool: mutation.run
    arguments: {}
    idempotency: null
"#;
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let service = WorkflowService::new(
            Arc::clone(&journal),
            repository,
            Arc::new(DenyWorkflowEffects),
        );
        service
            .register_definition(NON_IDEMPOTENT, "test")
            .expect("register");
        let queued = service
            .queue_run("crashy", "1.0.0", json!({}))
            .expect("queue");
        service
            .append_run_event(&queued.run_id, "workflow.run.started.v1", json!({}))
            .expect("claim");
        service
            .append_run_event(
                &queued.run_id,
                "workflow.step.started.v1",
                json!({"step_id": "mutate", "attempt": 1}),
            )
            .expect("started effect");
        let recovered = service.recover_interrupted().expect("recover");
        assert_eq!(recovered.len(), 1);
        assert_eq!(
            recovered[0].status,
            colossus_contracts::WorkflowStatus::Interrupted
        );
        let events = journal
            .read_stream(&format!("workflow-run:{}", queued.run_id))
            .expect("events");
        assert!(
            events
                .iter()
                .any(|event| { event.event_type == "workflow.step.outcome_unknown.v1" })
        );
        assert!(
            service
                .resume_run(&queued.run_id)
                .await
                .expect_err("unsafe retry")
                .to_string()
                .contains("cannot be retried")
        );
        assert!(service.drain().await.expect("drain").is_empty());

        let completed_before_crash = service
            .queue_run("crashy", "1.0.0", json!({}))
            .expect("second queue");
        service
            .append_run_event(
                &completed_before_crash.run_id,
                "workflow.run.started.v1",
                json!({}),
            )
            .expect("second claim");
        service
            .append_run_event(
                &completed_before_crash.run_id,
                "workflow.step.started.v1",
                json!({"step_id": "mutate", "attempt": 1}),
            )
            .expect("second start");
        service
            .append_run_event(
                &completed_before_crash.run_id,
                "workflow.step.completed.v1",
                json!({"step_id": "mutate", "root_index": 0, "output": {}}),
            )
            .expect("durable completion");
        service.recover_interrupted().expect("second recover");
        let completed_events = journal
            .read_stream(&format!("workflow-run:{}", completed_before_crash.run_id))
            .expect("second events");
        assert!(
            !completed_events
                .iter()
                .any(|event| { event.event_type == "workflow.step.outcome_unknown.v1" })
        );
    }
}
