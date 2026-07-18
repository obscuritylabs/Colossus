//! Strict YAML workflow validation and event-sourced durable run service.

use async_recursion::async_recursion;
use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, CredentialReference, EventClassification, EventEnvelope, ExecutionContext,
    NewEvent, WorkflowDefinition, WorkflowRun, WorkflowSchedule, WorkflowScheduleDispatch,
    WorkflowScheduleDispatchStatus, WorkflowScheduleMisfirePolicy, WorkflowStatus, WorkflowStep,
    WorkflowSubscription, WorkflowSubscriptionDelivery, WorkflowSubscriptionDispatch,
    WorkflowSubscriptionDispatchStatus, WorkflowTriggerKind, WorkflowWebhook,
    WorkflowWebhookDelivery, WorkflowWebhookDispatch,
};
use colossus_ports::{EventJournal, StoreError, WorkflowRepository};
use futures::{StreamExt as _, TryStreamExt as _, stream};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};
use thiserror::Error;
use time::{
    Duration as TimeDuration, OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339,
};
use tokio::sync::Semaphore;
use uuid::Uuid;

const MAX_WORKFLOW_BYTES: usize = 1024 * 1024;
const MAX_CONCURRENCY: u32 = 64;
const MAX_STEP_BUDGET: u32 = 10_000;
const MAX_FOREACH_ITEMS: u32 = 1_000;
const MAX_WORKFLOW_CALL_DEPTH: usize = 16;
const MAX_CONDITION_BYTES: usize = 16 * 1024;
const MAX_CONDITION_TOKENS: usize = 4_096;
const MAX_CONDITION_DEPTH: usize = 128;
const MIN_SCHEDULE_CADENCE_SECONDS: u64 = 60;
const MAX_SCHEDULE_CADENCE_SECONDS: u64 = 31 * 24 * 60 * 60;
const MAX_WORKFLOW_SCHEDULES: usize = 10_000;
const MAX_SCHEDULE_ID_BYTES: usize = 128;
const MAX_WORKFLOW_WEBHOOKS: usize = 10_000;
const MAX_WEBHOOK_ID_BYTES: usize = 128;
const MAX_WEBHOOK_DELIVERY_ID_BYTES: usize = 128;
const MIN_WEBHOOK_REPLAY_WINDOW_SECONDS: u64 = 60;
const MAX_WEBHOOK_REPLAY_WINDOW_SECONDS: u64 = 60 * 60;
const MAX_WEBHOOK_BODY_BYTES: u64 = 1024 * 1024;
const MAX_WEBHOOK_HEADERS: usize = 64;
const MAX_WEBHOOK_HEADER_BYTES: usize = 32 * 1024;
const MAX_WORKFLOW_SUBSCRIPTIONS: usize = 10_000;
const MAX_SUBSCRIPTION_ID_BYTES: usize = 128;
const MAX_SUBSCRIPTION_EVENT_TYPE_BYTES: usize = 256;
const MAX_SUBSCRIPTION_STREAM_PREFIX_BYTES: usize = 256;
const MAX_SUBSCRIPTION_SCAN_EVENTS: usize = 256;
const MAX_SUBSCRIPTION_DISPATCHES_PER_TICK: usize = 64;

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

fn scoped_execution_id(scope: &str, step_id: &str) -> String {
    if scope.is_empty() {
        step_id.into()
    } else {
        format!("{scope}/{step_id}")
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
        if source.len() > MAX_CONDITION_BYTES {
            return Err(WorkflowError::InvalidDefinition(format!(
                "condition exceeds {MAX_CONDITION_BYTES} bytes"
            )));
        }
        let tokens = tokenize(source)?;
        if tokens.len() > MAX_CONDITION_TOKENS {
            return Err(WorkflowError::InvalidDefinition(format!(
                "condition exceeds {MAX_CONDITION_TOKENS} tokens"
            )));
        }
        let mut parser = Parser {
            tokens,
            position: 0,
            depth: 0,
            complexity: 0,
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
    depth: usize,
    complexity: usize,
}

impl Parser {
    fn add_boolean_node(&mut self) -> Result<(), WorkflowError> {
        if self.complexity >= MAX_CONDITION_DEPTH {
            return Err(WorkflowError::InvalidDefinition(format!(
                "condition boolean complexity exceeds {MAX_CONDITION_DEPTH} nodes"
            )));
        }
        self.complexity += 1;
        Ok(())
    }

    fn nested<T>(
        &mut self,
        parse: impl FnOnce(&mut Self) -> Result<T, WorkflowError>,
    ) -> Result<T, WorkflowError> {
        if self.depth >= MAX_CONDITION_DEPTH {
            return Err(WorkflowError::InvalidDefinition(format!(
                "condition nesting exceeds {MAX_CONDITION_DEPTH} levels"
            )));
        }
        self.depth += 1;
        let result = parse(self);
        self.depth -= 1;
        result
    }

    fn parse_or(&mut self) -> Result<Expr, WorkflowError> {
        let mut expression = self.parse_and()?;
        while self.consume(&Token::Or) {
            self.add_boolean_node()?;
            expression = Expr::Or(Box::new(expression), Box::new(self.parse_and()?));
        }
        Ok(expression)
    }

    fn parse_and(&mut self) -> Result<Expr, WorkflowError> {
        let mut expression = self.parse_unary()?;
        while self.consume(&Token::And) {
            self.add_boolean_node()?;
            expression = Expr::And(Box::new(expression), Box::new(self.parse_unary()?));
        }
        Ok(expression)
    }

    fn parse_unary(&mut self) -> Result<Expr, WorkflowError> {
        if self.consume(&Token::Not) {
            self.add_boolean_node()?;
            return self
                .nested(Self::parse_unary)
                .map(|expression| Expr::Not(Box::new(expression)));
        }
        if self.consume(&Token::LParen) {
            let expression = self.nested(Self::parse_or)?;
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

    fn create_schedule(
        &self,
        schedule: &WorkflowSchedule,
        actor: Actor,
    ) -> Result<WorkflowSchedule, StoreError> {
        let stream_id = schedule_stream(&schedule.schedule_id);
        if !self.journal.read_stream(&stream_id)?.is_empty() {
            return Err(StoreError::Adapter(format!(
                "workflow schedule {} already exists",
                schedule.schedule_id
            )));
        }
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: 0,
            classification: EventClassification::Workflow,
            event_type: "workflow.schedule.registered.v1".into(),
            actor,
            context: ExecutionContext {
                correlation_id: schedule.schedule_id.clone(),
                workflow_id: Some(schedule.schedule_id.clone()),
                workflow_hash: Some(schedule.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": schedule}),
        })?;
        Ok(schedule.clone())
    }

    fn set_schedule_enabled(
        &self,
        schedule_id: &str,
        enabled: bool,
        updated_at: &str,
        actor: Actor,
    ) -> Result<WorkflowSchedule, StoreError> {
        let mut schedule = self
            .schedule(schedule_id)?
            .ok_or_else(|| StoreError::NotFound(format!("workflow schedule {schedule_id}")))?;
        if schedule.enabled == enabled {
            return Ok(schedule);
        }
        schedule.enabled = enabled;
        schedule.updated_at = updated_at.into();
        if enabled {
            schedule.blocked_reason = None;
        }
        let stream_id = schedule_stream(schedule_id);
        let expected_stream_version = u64::try_from(self.journal.read_stream(&stream_id)?.len())
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: EventClassification::Workflow,
            event_type: if enabled {
                "workflow.schedule.enabled.v1"
            } else {
                "workflow.schedule.disabled.v1"
            }
            .into(),
            actor,
            context: ExecutionContext {
                correlation_id: schedule_id.into(),
                workflow_id: Some(schedule_id.into()),
                workflow_hash: Some(schedule.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": &schedule}),
        })?;
        Ok(schedule)
    }

    fn schedule(&self, schedule_id: &str) -> Result<Option<WorkflowSchedule>, StoreError> {
        fold_schedule(self.journal.as_ref(), schedule_id)
    }

    fn schedules(&self, limit: usize) -> Result<Vec<WorkflowSchedule>, StoreError> {
        let mut schedule_ids = BTreeSet::new();
        for event in self.journal.read_global(1, usize::MAX)? {
            if event.event_type == "workflow.schedule.registered.v1"
                && let Some(schedule_id) = event.stream_id.strip_prefix("workflow-schedule:")
            {
                schedule_ids.insert(schedule_id.to_owned());
            }
        }
        schedule_ids
            .into_iter()
            .take(limit)
            .map(|schedule_id| {
                fold_schedule(self.journal.as_ref(), &schedule_id)?.ok_or_else(|| {
                    StoreError::Verification(format!(
                        "workflow schedule {schedule_id} cannot be reconstructed"
                    ))
                })
            })
            .collect()
    }

    fn create_webhook(
        &self,
        webhook: &WorkflowWebhook,
        actor: Actor,
    ) -> Result<WorkflowWebhook, StoreError> {
        let stream_id = webhook_stream(&webhook.webhook_id);
        if !self.journal.read_stream(&stream_id)?.is_empty() {
            return Err(StoreError::Adapter(format!(
                "workflow webhook {} already exists",
                webhook.webhook_id
            )));
        }
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: 0,
            classification: EventClassification::Workflow,
            event_type: "workflow.webhook.registered.v1".into(),
            actor,
            context: ExecutionContext {
                correlation_id: webhook.webhook_id.clone(),
                workflow_id: Some(webhook.webhook_id.clone()),
                workflow_hash: Some(webhook.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": webhook}),
        })?;
        Ok(webhook.clone())
    }

    fn set_webhook_enabled(
        &self,
        webhook_id: &str,
        enabled: bool,
        updated_at: &str,
        actor: Actor,
    ) -> Result<WorkflowWebhook, StoreError> {
        let mut webhook = self
            .webhook(webhook_id)?
            .ok_or_else(|| StoreError::NotFound(format!("workflow webhook {webhook_id}")))?;
        if webhook.enabled == enabled {
            return Ok(webhook);
        }
        webhook.enabled = enabled;
        webhook.updated_at = updated_at.into();
        if enabled {
            webhook.blocked_reason = None;
        }
        let stream_id = webhook_stream(webhook_id);
        let expected_stream_version = u64::try_from(self.journal.read_stream(&stream_id)?.len())
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: EventClassification::Workflow,
            event_type: if enabled {
                "workflow.webhook.enabled.v1"
            } else {
                "workflow.webhook.disabled.v1"
            }
            .into(),
            actor,
            context: ExecutionContext {
                correlation_id: webhook_id.into(),
                workflow_id: Some(webhook_id.into()),
                workflow_hash: Some(webhook.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": &webhook}),
        })?;
        Ok(webhook)
    }

    fn webhook(&self, webhook_id: &str) -> Result<Option<WorkflowWebhook>, StoreError> {
        fold_webhook(self.journal.as_ref(), webhook_id)
    }

    fn webhooks(&self, limit: usize) -> Result<Vec<WorkflowWebhook>, StoreError> {
        let mut webhook_ids = BTreeSet::new();
        for event in self.journal.read_global(1, usize::MAX)? {
            if event.event_type == "workflow.webhook.registered.v1"
                && let Some(webhook_id) = event.stream_id.strip_prefix("workflow-webhook:")
            {
                webhook_ids.insert(webhook_id.to_owned());
            }
        }
        webhook_ids
            .into_iter()
            .take(limit)
            .map(|webhook_id| {
                fold_webhook(self.journal.as_ref(), &webhook_id)?.ok_or_else(|| {
                    StoreError::Verification(format!(
                        "workflow webhook {webhook_id} cannot be reconstructed"
                    ))
                })
            })
            .collect()
    }

    fn webhook_delivery(
        &self,
        webhook_id: &str,
        delivery_id: &str,
    ) -> Result<Option<WorkflowWebhookDelivery>, StoreError> {
        fold_webhook_delivery(self.journal.as_ref(), webhook_id, delivery_id)
    }

    fn create_subscription(
        &self,
        subscription: &WorkflowSubscription,
        actor: Actor,
    ) -> Result<WorkflowSubscription, StoreError> {
        let stream_id = subscription_stream(&subscription.subscription_id);
        if !self.journal.read_stream(&stream_id)?.is_empty() {
            return Err(StoreError::Adapter(format!(
                "workflow subscription {} already exists",
                subscription.subscription_id
            )));
        }
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: 0,
            classification: EventClassification::Workflow,
            event_type: "workflow.subscription.registered.v1".into(),
            actor,
            context: ExecutionContext {
                correlation_id: subscription.subscription_id.clone(),
                workflow_id: Some(subscription.subscription_id.clone()),
                workflow_hash: Some(subscription.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": subscription}),
        })?;
        Ok(subscription.clone())
    }

    fn set_subscription_enabled(
        &self,
        subscription_id: &str,
        enabled: bool,
        updated_at: &str,
        actor: Actor,
    ) -> Result<WorkflowSubscription, StoreError> {
        let mut subscription = self.subscription(subscription_id)?.ok_or_else(|| {
            StoreError::NotFound(format!("workflow subscription {subscription_id}"))
        })?;
        if subscription.enabled == enabled {
            return Ok(subscription);
        }
        subscription.enabled = enabled;
        subscription.updated_at = updated_at.into();
        if enabled {
            subscription.blocked_reason = None;
        }
        let stream_id = subscription_stream(subscription_id);
        let expected_stream_version = u64::try_from(self.journal.read_stream(&stream_id)?.len())
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: EventClassification::Workflow,
            event_type: if enabled {
                "workflow.subscription.enabled.v1"
            } else {
                "workflow.subscription.disabled.v1"
            }
            .into(),
            actor,
            context: ExecutionContext {
                correlation_id: subscription_id.into(),
                workflow_id: Some(subscription_id.into()),
                workflow_hash: Some(subscription.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": &subscription}),
        })?;
        Ok(subscription)
    }

    fn subscription(
        &self,
        subscription_id: &str,
    ) -> Result<Option<WorkflowSubscription>, StoreError> {
        fold_subscription(self.journal.as_ref(), subscription_id)
    }

    fn subscriptions(&self, limit: usize) -> Result<Vec<WorkflowSubscription>, StoreError> {
        let mut subscription_ids = BTreeSet::new();
        for event in self.journal.read_global(1, usize::MAX)? {
            if event.event_type == "workflow.subscription.registered.v1"
                && let Some(subscription_id) =
                    event.stream_id.strip_prefix("workflow-subscription:")
            {
                subscription_ids.insert(subscription_id.to_owned());
            }
        }
        subscription_ids
            .into_iter()
            .take(limit)
            .map(|subscription_id| {
                fold_subscription(self.journal.as_ref(), &subscription_id)?.ok_or_else(|| {
                    StoreError::Verification(format!(
                        "workflow subscription {subscription_id} cannot be reconstructed"
                    ))
                })
            })
            .collect()
    }

    fn subscription_delivery(
        &self,
        subscription_id: &str,
        source_event_id: &str,
    ) -> Result<Option<WorkflowSubscriptionDelivery>, StoreError> {
        fold_subscription_delivery(self.journal.as_ref(), subscription_id, source_event_id)
    }
}

fn schedule_stream(schedule_id: &str) -> String {
    format!("workflow-schedule:{schedule_id}")
}

fn fold_schedule(
    journal: &dyn EventJournal,
    schedule_id: &str,
) -> Result<Option<WorkflowSchedule>, StoreError> {
    let events = journal.read_stream(&schedule_stream(schedule_id))?;
    let Some(last) = events.last() else {
        return Ok(None);
    };
    let payload = journal.decrypt_payload(last)?;
    let schedule: WorkflowSchedule = serde_json::from_value(
        payload
            .get("record")
            .cloned()
            .ok_or_else(|| StoreError::Verification("schedule record is absent".into()))?,
    )
    .map_err(|error| StoreError::Verification(error.to_string()))?;
    if schedule.schedule_id != schedule_id {
        return Err(StoreError::Verification(format!(
            "schedule stream {schedule_id} contains record {}",
            schedule.schedule_id
        )));
    }
    Ok(Some(schedule))
}

fn webhook_stream(webhook_id: &str) -> String {
    format!("workflow-webhook:{webhook_id}")
}

fn webhook_delivery_stream(webhook_id: &str, delivery_id: &str) -> String {
    let digest = hex::encode(Sha256::digest(delivery_id.as_bytes()));
    format!("workflow-webhook-delivery:{webhook_id}:{digest}")
}

fn fold_webhook(
    journal: &dyn EventJournal,
    webhook_id: &str,
) -> Result<Option<WorkflowWebhook>, StoreError> {
    let events = journal.read_stream(&webhook_stream(webhook_id))?;
    let Some(last) = events.last() else {
        return Ok(None);
    };
    let payload = journal.decrypt_payload(last)?;
    let webhook: WorkflowWebhook = serde_json::from_value(
        payload
            .get("record")
            .cloned()
            .ok_or_else(|| StoreError::Verification("webhook record is absent".into()))?,
    )
    .map_err(|error| StoreError::Verification(error.to_string()))?;
    if webhook.webhook_id != webhook_id {
        return Err(StoreError::Verification(format!(
            "webhook stream {webhook_id} contains record {}",
            webhook.webhook_id
        )));
    }
    Ok(Some(webhook))
}

fn fold_webhook_delivery(
    journal: &dyn EventJournal,
    webhook_id: &str,
    delivery_id: &str,
) -> Result<Option<WorkflowWebhookDelivery>, StoreError> {
    let events = journal.read_stream(&webhook_delivery_stream(webhook_id, delivery_id))?;
    let Some(first) = events.first() else {
        return Ok(None);
    };
    let payload = journal.decrypt_payload(first)?;
    let delivery: WorkflowWebhookDelivery = serde_json::from_value(
        payload
            .get("record")
            .cloned()
            .ok_or_else(|| StoreError::Verification("webhook delivery record is absent".into()))?,
    )
    .map_err(|error| StoreError::Verification(error.to_string()))?;
    if delivery.webhook_id != webhook_id || delivery.delivery_id != delivery_id {
        return Err(StoreError::Verification(
            "webhook delivery stream identity does not match its record".into(),
        ));
    }
    Ok(Some(delivery))
}

fn subscription_stream(subscription_id: &str) -> String {
    format!("workflow-subscription:{subscription_id}")
}

fn subscription_delivery_stream(subscription_id: &str, source_event_id: &str) -> String {
    let digest = hex::encode(Sha256::digest(source_event_id.as_bytes()));
    format!("workflow-subscription-delivery:{subscription_id}:{digest}")
}

fn fold_subscription(
    journal: &dyn EventJournal,
    subscription_id: &str,
) -> Result<Option<WorkflowSubscription>, StoreError> {
    let events = journal.read_stream(&subscription_stream(subscription_id))?;
    let Some(last) = events.last() else {
        return Ok(None);
    };
    let payload = journal.decrypt_payload(last)?;
    let subscription: WorkflowSubscription = serde_json::from_value(
        payload
            .get("record")
            .cloned()
            .ok_or_else(|| StoreError::Verification("subscription record is absent".into()))?,
    )
    .map_err(|error| StoreError::Verification(error.to_string()))?;
    if subscription.subscription_id != subscription_id {
        return Err(StoreError::Verification(format!(
            "subscription stream {subscription_id} contains record {}",
            subscription.subscription_id
        )));
    }
    Ok(Some(subscription))
}

fn fold_subscription_delivery(
    journal: &dyn EventJournal,
    subscription_id: &str,
    source_event_id: &str,
) -> Result<Option<WorkflowSubscriptionDelivery>, StoreError> {
    let events = journal.read_stream(&subscription_delivery_stream(
        subscription_id,
        source_event_id,
    ))?;
    let Some(first) = events.first() else {
        return Ok(None);
    };
    let payload = journal.decrypt_payload(first)?;
    let delivery: WorkflowSubscriptionDelivery =
        serde_json::from_value(payload.get("record").cloned().ok_or_else(|| {
            StoreError::Verification("subscription delivery record is absent".into())
        })?)
        .map_err(|error| StoreError::Verification(error.to_string()))?;
    if delivery.subscription_id != subscription_id || delivery.source_event_id != source_event_id {
        return Err(StoreError::Verification(
            "subscription delivery stream identity does not match its record".into(),
        ));
    }
    Ok(Some(delivery))
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
        parent_run_id: start
            .get("parent_run_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        parent_step_id: start
            .get("parent_step_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        parent_execution_id: start
            .get("parent_execution_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        trigger_kind: start
            .get("trigger_kind")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| StoreError::Verification(error.to_string()))?,
        trigger_id: start
            .get("trigger_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        trigger_occurrence: start
            .get("trigger_occurrence")
            .and_then(Value::as_str)
            .map(str::to_owned),
        call_depth: start
            .get("call_depth")
            .and_then(Value::as_u64)
            .and_then(|depth| u16::try_from(depth).ok())
            .unwrap_or(1),
        status: if first.event_type == "workflow.run.queued.v1" {
            WorkflowStatus::Queued
        } else {
            WorkflowStatus::Running
        },
        inputs: start.get("inputs").cloned().unwrap_or(Value::Null),
        outputs: None,
        completed_steps: 0,
        waiting_step_id: None,
        waiting_execution_id: None,
        waiting_reason: None,
        waiting_child_run_id: None,
    };
    for event in events.iter().skip(1) {
        let payload = journal.decrypt_payload(event)?;
        match event.event_type.as_str() {
            "workflow.run.queued.v1" => run.status = WorkflowStatus::Queued,
            "workflow.run.started.v1" => {
                run.status = WorkflowStatus::Running;
                run.waiting_step_id = None;
                run.waiting_execution_id = None;
                run.waiting_reason = None;
                run.waiting_child_run_id = None;
            }
            "workflow.step.completed.v1" => {
                run.completed_steps = payload
                    .get("root_index")
                    .and_then(Value::as_u64)
                    .and_then(|index| u32::try_from(index.saturating_add(1)).ok())
                    .unwrap_or(run.completed_steps);
            }
            "workflow.run.waiting.v1" => {
                run.status = WorkflowStatus::Waiting;
                run.waiting_step_id = payload
                    .get("step_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                run.waiting_execution_id = payload
                    .get("execution_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                run.waiting_reason = payload
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                run.waiting_child_run_id = payload
                    .get("child_run_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            "workflow.run.resumed.v1" => {
                run.status = WorkflowStatus::Running;
                run.waiting_step_id = None;
                run.waiting_execution_id = None;
                run.waiting_reason = None;
                run.waiting_child_run_id = None;
            }
            "workflow.run.completed.v1" => {
                run.status = WorkflowStatus::Completed;
                run.outputs = payload.get("outputs").cloned();
                run.waiting_step_id = None;
                run.waiting_execution_id = None;
                run.waiting_reason = None;
                run.waiting_child_run_id = None;
            }
            "workflow.run.failed.v1" => {
                run.status = WorkflowStatus::Failed;
                run.waiting_step_id = None;
                run.waiting_execution_id = None;
                run.waiting_reason = None;
                run.waiting_child_run_id = None;
            }
            "workflow.run.cancelled.v1" => {
                run.status = WorkflowStatus::Cancelled;
                run.waiting_step_id = None;
                run.waiting_execution_id = None;
                run.waiting_reason = None;
                run.waiting_child_run_id = None;
            }
            "workflow.run.interrupted.v1" => {
                run.status = WorkflowStatus::Interrupted;
                run.waiting_step_id = None;
                run.waiting_execution_id = None;
                run.waiting_reason = None;
                run.waiting_child_run_id = None;
            }
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

fn parse_schedule_time(value: &str, label: &str) -> Result<OffsetDateTime, WorkflowError> {
    let parsed = OffsetDateTime::parse(value, &Rfc3339).map_err(|error| {
        WorkflowError::InvalidDefinition(format!("{label} must be UTC RFC3339: {error}"))
    })?;
    if parsed.offset() != UtcOffset::UTC {
        return Err(WorkflowError::InvalidDefinition(format!(
            "{label} must use the UTC Z offset"
        )));
    }
    Ok(parsed)
}

fn format_schedule_time(value: OffsetDateTime) -> Result<String, WorkflowError> {
    value
        .to_offset(UtcOffset::UTC)
        .format(&Rfc3339)
        .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))
}

fn add_schedule_occurrences(
    base: OffsetDateTime,
    cadence_seconds: u64,
    occurrences: u64,
) -> Result<OffsetDateTime, WorkflowError> {
    let total_seconds = cadence_seconds
        .checked_mul(occurrences)
        .ok_or_else(|| WorkflowError::InvalidTransition("schedule cadence overflow".into()))?;
    let total_seconds = i64::try_from(total_seconds)
        .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?;
    base.checked_add(TimeDuration::seconds(total_seconds))
        .ok_or_else(|| WorkflowError::InvalidTransition("schedule timestamp overflow".into()))
}

fn scheduled_run_id(schedule_id: &str, occurrence: &str) -> String {
    let digest = hex::encode(Sha256::digest(
        format!("{schedule_id}\0{occurrence}").as_bytes(),
    ));
    format!("schedule-{}", digest.chars().take(32).collect::<String>())
}

fn schedule_event(
    schedule: &WorkflowSchedule,
    expected_stream_version: u64,
    event_type: &str,
    payload: Value,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: schedule_stream(&schedule.schedule_id),
        expected_stream_version,
        classification: EventClassification::Workflow,
        event_type: event_type.into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: schedule.schedule_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: schedule.schedule_id.clone(),
            workflow_id: Some(schedule.schedule_id.clone()),
            workflow_hash: Some(schedule.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload,
    }
}

fn scheduled_run_event(schedule: &WorkflowSchedule, run_id: &str, occurrence: &str) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: format!("workflow-run:{run_id}"),
        expected_stream_version: 0,
        classification: EventClassification::Workflow,
        event_type: "workflow.run.queued.v1".into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: schedule.schedule_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: run_id.into(),
            run_id: Some(run_id.into()),
            workflow_id: Some(run_id.into()),
            workflow_hash: Some(schedule.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload: json!({
            "workflow_name": schedule.workflow_name,
            "workflow_version": schedule.workflow_version,
            "workflow_hash": schedule.workflow_hash,
            "inputs": schedule.inputs,
            "parent_run_id": Value::Null,
            "parent_step_id": Value::Null,
            "parent_execution_id": Value::Null,
            "trigger_kind": WorkflowTriggerKind::Schedule,
            "trigger_id": schedule.schedule_id,
            "trigger_occurrence": occurrence,
            "call_depth": 1,
        }),
    }
}

fn valid_environment_reference(reference: &str) -> bool {
    reference.strip_prefix("env:").is_some_and(|variable| {
        !variable.is_empty()
            && variable.len() <= 128
            && variable
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            && variable
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_uppercase)
    })
}

fn validate_webhook_headers(headers: &BTreeMap<String, String>) -> Result<(), WorkflowError> {
    if headers.len() > MAX_WEBHOOK_HEADERS {
        return Err(WorkflowError::InvalidDefinition(format!(
            "webhook headers exceed the {MAX_WEBHOOK_HEADERS} field limit"
        )));
    }
    let mut total = 0_usize;
    for (name, value) in headers {
        if name.is_empty()
            || name.len() > 256
            || !name.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(
                        byte,
                        b'!' | b'#'
                            | b'$'
                            | b'%'
                            | b'&'
                            | b'\''
                            | b'*'
                            | b'+'
                            | b'-'
                            | b'.'
                            | b'^'
                            | b'_'
                            | b'`'
                            | b'|'
                            | b'~'
                    )
            })
        {
            return Err(WorkflowError::InvalidDefinition(
                "webhook header names must be lowercase HTTP field names".into(),
            ));
        }
        if value.len() > 8 * 1024 || value.chars().any(|character| character.is_control()) {
            return Err(WorkflowError::InvalidDefinition(format!(
                "webhook header {name} contains an invalid or oversized value"
            )));
        }
        total = total.checked_add(name.len() + value.len()).ok_or_else(|| {
            WorkflowError::InvalidDefinition("webhook header size overflow".into())
        })?;
    }
    if total > MAX_WEBHOOK_HEADER_BYTES {
        return Err(WorkflowError::InvalidDefinition(format!(
            "webhook headers exceed {MAX_WEBHOOK_HEADER_BYTES} bytes"
        )));
    }
    Ok(())
}

fn verify_webhook_signature(
    timestamp: &str,
    delivery_id: &str,
    body: &[u8],
    signature: &str,
    secret: &[u8],
) -> Result<(), WorkflowError> {
    let signature = signature.strip_prefix("sha256=").unwrap_or(signature);
    if signature.len() != 64
        || !signature
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(WorkflowError::InvalidDefinition(
            "webhook signature must be sha256=<64 lowercase hex characters>".into(),
        ));
    }
    let decoded = hex::decode(signature).map_err(|_| {
        WorkflowError::InvalidDefinition("webhook signature is not valid hexadecimal".into())
    })?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret)
        .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?;
    mac.update(timestamp.as_bytes());
    mac.update(b"\n");
    mac.update(delivery_id.as_bytes());
    mac.update(b"\n");
    mac.update(body);
    mac.verify_slice(&decoded)
        .map_err(|_| WorkflowError::InvalidTransition("webhook signature is invalid".into()))
}

fn webhook_run_id(webhook_id: &str, delivery_id: &str) -> String {
    let digest = hex::encode(Sha256::digest(
        format!("{webhook_id}\0{delivery_id}").as_bytes(),
    ));
    format!("webhook-{}", digest.chars().take(32).collect::<String>())
}

fn webhook_delivery_event(
    webhook: &WorkflowWebhook,
    delivery: &WorkflowWebhookDelivery,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: webhook_delivery_stream(&webhook.webhook_id, &delivery.delivery_id),
        expected_stream_version: 0,
        classification: EventClassification::Workflow,
        event_type: "workflow.webhook.delivery.accepted.v1".into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: webhook.webhook_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: delivery.run_id.clone(),
            run_id: Some(delivery.run_id.clone()),
            workflow_id: Some(webhook.webhook_id.clone()),
            workflow_hash: Some(webhook.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload: json!({"record": delivery}),
    }
}

fn webhook_run_event(
    webhook: &WorkflowWebhook,
    run_id: &str,
    delivery_id: &str,
    inputs: Value,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: format!("workflow-run:{run_id}"),
        expected_stream_version: 0,
        classification: EventClassification::Workflow,
        event_type: "workflow.run.queued.v1".into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: webhook.webhook_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: run_id.into(),
            run_id: Some(run_id.into()),
            workflow_id: Some(run_id.into()),
            workflow_hash: Some(webhook.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload: json!({
            "workflow_name": webhook.workflow_name,
            "workflow_version": webhook.workflow_version,
            "workflow_hash": webhook.workflow_hash,
            "inputs": inputs,
            "parent_run_id": Value::Null,
            "parent_step_id": Value::Null,
            "parent_execution_id": Value::Null,
            "trigger_kind": WorkflowTriggerKind::Webhook,
            "trigger_id": webhook.webhook_id,
            "trigger_occurrence": delivery_id,
            "call_depth": 1,
        }),
    }
}

fn valid_subscription_event_type(event_type: &str) -> bool {
    let Some((name, version)) = event_type.rsplit_once(".v") else {
        return false;
    };
    !name.is_empty()
        && !version.is_empty()
        && version.bytes().all(|byte| byte.is_ascii_digit())
        && event_type.len() <= MAX_SUBSCRIPTION_EVENT_TYPE_BYTES
        && !event_type.starts_with("workflow.")
        && event_type.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn subscription_matches(subscription: &WorkflowSubscription, event: &EventEnvelope) -> bool {
    event.classification == EventClassification::Domain
        && event.event_type == subscription.event_type
        && subscription
            .stream_prefix
            .as_deref()
            .is_none_or(|prefix| event.stream_id.starts_with(prefix))
}

fn subscription_run_id(subscription_id: &str, source_event_id: &str) -> String {
    let digest = hex::encode(Sha256::digest(
        format!("{subscription_id}\0{source_event_id}").as_bytes(),
    ));
    format!(
        "subscription-{}",
        digest.chars().take(32).collect::<String>()
    )
}

fn subscription_event(
    subscription: &WorkflowSubscription,
    expected_stream_version: u64,
    event_type: &str,
    payload: Value,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: subscription_stream(&subscription.subscription_id),
        expected_stream_version,
        classification: EventClassification::Workflow,
        event_type: event_type.into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: subscription.subscription_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: subscription.subscription_id.clone(),
            workflow_id: Some(subscription.subscription_id.clone()),
            workflow_hash: Some(subscription.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload,
    }
}

fn subscription_delivery_event(
    subscription: &WorkflowSubscription,
    delivery: &WorkflowSubscriptionDelivery,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: subscription_delivery_stream(
            &subscription.subscription_id,
            &delivery.source_event_id,
        ),
        expected_stream_version: 0,
        classification: EventClassification::Workflow,
        event_type: "workflow.subscription.delivery.accepted.v1".into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: subscription.subscription_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: delivery.run_id.clone(),
            run_id: Some(delivery.run_id.clone()),
            workflow_id: Some(subscription.subscription_id.clone()),
            workflow_hash: Some(subscription.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload: json!({"record": delivery}),
    }
}

fn subscription_run_event(
    subscription: &WorkflowSubscription,
    source_event_id: &str,
    run_id: &str,
    inputs: Value,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: format!("workflow-run:{run_id}"),
        expected_stream_version: 0,
        classification: EventClassification::Workflow,
        event_type: "workflow.run.queued.v1".into(),
        actor: Actor {
            actor_type: ActorType::Workflow,
            id: subscription.subscription_id.clone(),
        },
        context: ExecutionContext {
            correlation_id: run_id.into(),
            run_id: Some(run_id.into()),
            workflow_id: Some(run_id.into()),
            workflow_hash: Some(subscription.workflow_hash.clone()),
            ..ExecutionContext::default()
        },
        payload: json!({
            "workflow_name": subscription.workflow_name,
            "workflow_version": subscription.workflow_version,
            "workflow_hash": subscription.workflow_hash,
            "inputs": inputs,
            "parent_run_id": Value::Null,
            "parent_step_id": Value::Null,
            "parent_execution_id": Value::Null,
            "trigger_kind": WorkflowTriggerKind::Subscription,
            "trigger_id": subscription.subscription_id,
            "trigger_occurrence": source_event_id,
            "call_depth": 1,
        }),
    }
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
    /// Late-bound credential references whose values are deliberately absent.
    pub credential_references: Vec<CredentialReference>,
    /// Workflow run identifier.
    pub run_id: String,
    /// Workflow step identifier.
    pub step_id: String,
    /// Static step identifier from the pinned definition.
    pub definition_step_id: String,
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
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
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
        self.queue_run_with_lineage(
            &Uuid::now_v7().to_string(),
            name,
            version,
            inputs,
            None,
            None,
            None,
            1,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn queue_run_with_lineage(
        &self,
        run_id: &str,
        name: &str,
        version: &str,
        inputs: Value,
        parent_run_id: Option<&str>,
        parent_step_id: Option<&str>,
        parent_execution_id: Option<&str>,
        call_depth: u16,
    ) -> Result<WorkflowRun, WorkflowError> {
        if usize::from(call_depth) > MAX_WORKFLOW_CALL_DEPTH {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow call depth exceeds {MAX_WORKFLOW_CALL_DEPTH}"
            )));
        }
        let (definition, workflow_hash) = self
            .repository
            .definition(name, version)?
            .ok_or_else(|| WorkflowError::NotFound(format!("{name}:{version}")))?;
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        validate_instance(&definition.inputs, &inputs, "input")?;
        self.append_run_event(
            run_id,
            "workflow.run.queued.v1",
            json!({
                "workflow_name": name,
                "workflow_version": version,
                "workflow_hash": workflow_hash,
                "inputs": inputs,
                "parent_run_id": parent_run_id,
                "parent_step_id": parent_step_id,
                "parent_execution_id": parent_execution_id,
                "call_depth": call_depth,
            }),
        )?;
        self.get_run(run_id)
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

    /// Create one bounded, hash-pinned cadence schedule.
    #[allow(clippy::too_many_arguments)]
    pub fn create_schedule(
        &self,
        schedule_id: &str,
        workflow_name: &str,
        workflow_version: &str,
        inputs: Value,
        cadence_seconds: u64,
        misfire_policy: WorkflowScheduleMisfirePolicy,
        enabled: bool,
        starts_at: Option<&str>,
    ) -> Result<WorkflowSchedule, WorkflowError> {
        let now = OffsetDateTime::now_utc();
        self.create_schedule_at(
            schedule_id,
            workflow_name,
            workflow_version,
            inputs,
            cadence_seconds,
            misfire_policy,
            enabled,
            starts_at,
            now,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_schedule_at(
        &self,
        schedule_id: &str,
        workflow_name: &str,
        workflow_version: &str,
        inputs: Value,
        cadence_seconds: u64,
        misfire_policy: WorkflowScheduleMisfirePolicy,
        enabled: bool,
        starts_at: Option<&str>,
        now: OffsetDateTime,
    ) -> Result<WorkflowSchedule, WorkflowError> {
        if schedule_id.is_empty()
            || schedule_id.len() > MAX_SCHEDULE_ID_BYTES
            || !valid_name(schedule_id)
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "schedule id must contain 1..={MAX_SCHEDULE_ID_BYTES} lowercase letters, digits, dots, or hyphens"
            )));
        }
        if !(MIN_SCHEDULE_CADENCE_SECONDS..=MAX_SCHEDULE_CADENCE_SECONDS).contains(&cadence_seconds)
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "schedule cadence must be between {MIN_SCHEDULE_CADENCE_SECONDS} and {MAX_SCHEDULE_CADENCE_SECONDS} seconds"
            )));
        }
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        if self.repository.schedule(schedule_id)?.is_some() {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow schedule {schedule_id} already exists"
            )));
        }
        if self.repository.schedules(MAX_WORKFLOW_SCHEDULES)?.len() >= MAX_WORKFLOW_SCHEDULES {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow schedule limit {MAX_WORKFLOW_SCHEDULES} is exhausted"
            )));
        }
        let (definition, workflow_hash) = self
            .repository
            .definition(workflow_name, workflow_version)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("{workflow_name}:{workflow_version}"))
            })?;
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        validate_instance(&definition.inputs, &inputs, "schedule input")?;
        let first_fire = match starts_at {
            Some(starts_at) => parse_schedule_time(starts_at, "schedule start")?,
            None => add_schedule_occurrences(now, cadence_seconds, 1)?,
        };
        let now = format_schedule_time(now)?;
        let starts_at = format_schedule_time(first_fire)?;
        let schedule = WorkflowSchedule {
            schedule_id: schedule_id.into(),
            workflow_name: workflow_name.into(),
            workflow_version: workflow_version.into(),
            workflow_hash,
            inputs,
            cadence_seconds,
            misfire_policy,
            enabled,
            starts_at: starts_at.clone(),
            next_fire_at: starts_at,
            last_scheduled_at: None,
            last_run_id: None,
            blocked_reason: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.repository.create_schedule(
            &schedule,
            Actor {
                actor_type: ActorType::User,
                id: "workflow-schedule-registrar".into(),
            },
        )?;
        Ok(schedule)
    }

    /// Reconstruct one canonical workflow schedule.
    pub fn get_schedule(&self, schedule_id: &str) -> Result<WorkflowSchedule, WorkflowError> {
        self.repository
            .schedule(schedule_id)?
            .ok_or_else(|| WorkflowError::NotFound(format!("workflow schedule {schedule_id}")))
    }

    /// List bounded schedules in deterministic identifier order.
    pub fn list_schedules(&self, limit: usize) -> Result<Vec<WorkflowSchedule>, WorkflowError> {
        self.repository
            .schedules(limit.min(MAX_WORKFLOW_SCHEDULES))
            .map_err(Into::into)
    }

    /// Explicitly enable or disable one schedule after rechecking pinned trust.
    pub fn set_schedule_enabled(
        &self,
        schedule_id: &str,
        enabled: bool,
    ) -> Result<WorkflowSchedule, WorkflowError> {
        let now = format_schedule_time(OffsetDateTime::now_utc())?;
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let schedule = self
            .repository
            .schedule(schedule_id)?
            .ok_or_else(|| WorkflowError::NotFound(format!("workflow schedule {schedule_id}")))?;
        if enabled {
            let (definition, current_hash) = self
                .repository
                .definition(&schedule.workflow_name, &schedule.workflow_version)?
                .ok_or_else(|| WorkflowError::NotFound(schedule.workflow_name.clone()))?;
            if current_hash != schedule.workflow_hash {
                return Err(WorkflowError::InvalidTransition(
                    "schedule cannot be enabled because its pinned workflow definition changed"
                        .into(),
                ));
            }
            validate_call_graph(self.repository.as_ref(), &definition, true)?;
            validate_instance(&definition.inputs, &schedule.inputs, "schedule input")?;
        }
        self.repository
            .set_schedule_enabled(
                schedule_id,
                enabled,
                &now,
                Actor {
                    actor_type: ActorType::User,
                    id: "workflow-schedule-operator".into(),
                },
            )
            .map_err(Into::into)
    }

    /// Create one bounded, hash-pinned authenticated workflow webhook.
    #[allow(clippy::too_many_arguments)]
    pub fn create_webhook(
        &self,
        webhook_id: &str,
        workflow_name: &str,
        workflow_version: &str,
        secret_reference: &str,
        replay_window_seconds: u64,
        max_body_bytes: u64,
        enabled: bool,
    ) -> Result<WorkflowWebhook, WorkflowError> {
        if webhook_id.is_empty()
            || webhook_id.len() > MAX_WEBHOOK_ID_BYTES
            || !valid_name(webhook_id)
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "webhook id must contain 1..={MAX_WEBHOOK_ID_BYTES} lowercase letters, digits, dots, or hyphens"
            )));
        }
        if !valid_environment_reference(secret_reference) {
            return Err(WorkflowError::InvalidDefinition(
                "webhook secret must use an env:VARIABLE credential reference".into(),
            ));
        }
        if !(MIN_WEBHOOK_REPLAY_WINDOW_SECONDS..=MAX_WEBHOOK_REPLAY_WINDOW_SECONDS)
            .contains(&replay_window_seconds)
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "webhook replay window must be between {MIN_WEBHOOK_REPLAY_WINDOW_SECONDS} and {MAX_WEBHOOK_REPLAY_WINDOW_SECONDS} seconds"
            )));
        }
        if !(1..=MAX_WEBHOOK_BODY_BYTES).contains(&max_body_bytes) {
            return Err(WorkflowError::InvalidDefinition(format!(
                "webhook body limit must be between 1 and {MAX_WEBHOOK_BODY_BYTES} bytes"
            )));
        }
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        if self.repository.webhook(webhook_id)?.is_some() {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow webhook {webhook_id} already exists"
            )));
        }
        if self.repository.webhooks(MAX_WORKFLOW_WEBHOOKS)?.len() >= MAX_WORKFLOW_WEBHOOKS {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow webhook limit {MAX_WORKFLOW_WEBHOOKS} is exhausted"
            )));
        }
        let (definition, workflow_hash) = self
            .repository
            .definition(workflow_name, workflow_version)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("{workflow_name}:{workflow_version}"))
            })?;
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        let now = format_schedule_time(OffsetDateTime::now_utc())?;
        let webhook = WorkflowWebhook {
            webhook_id: webhook_id.into(),
            workflow_name: workflow_name.into(),
            workflow_version: workflow_version.into(),
            workflow_hash,
            secret_reference: secret_reference.into(),
            enabled,
            replay_window_seconds,
            max_body_bytes,
            blocked_reason: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.repository.create_webhook(
            &webhook,
            Actor {
                actor_type: ActorType::User,
                id: "workflow-webhook-registrar".into(),
            },
        )?;
        Ok(webhook)
    }

    /// Reconstruct one canonical workflow webhook.
    pub fn get_webhook(&self, webhook_id: &str) -> Result<WorkflowWebhook, WorkflowError> {
        self.repository
            .webhook(webhook_id)?
            .ok_or_else(|| WorkflowError::NotFound(format!("workflow webhook {webhook_id}")))
    }

    /// List bounded workflow webhooks in deterministic identifier order.
    pub fn list_webhooks(&self, limit: usize) -> Result<Vec<WorkflowWebhook>, WorkflowError> {
        self.repository
            .webhooks(limit.min(MAX_WORKFLOW_WEBHOOKS))
            .map_err(Into::into)
    }

    /// Explicitly enable or disable one webhook after rechecking pinned trust.
    pub fn set_webhook_enabled(
        &self,
        webhook_id: &str,
        enabled: bool,
    ) -> Result<WorkflowWebhook, WorkflowError> {
        let now = format_schedule_time(OffsetDateTime::now_utc())?;
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let webhook = self
            .repository
            .webhook(webhook_id)?
            .ok_or_else(|| WorkflowError::NotFound(format!("workflow webhook {webhook_id}")))?;
        if enabled {
            self.validate_webhook_trust(&webhook)?;
        }
        self.repository
            .set_webhook_enabled(
                webhook_id,
                enabled,
                &now,
                Actor {
                    actor_type: ActorType::User,
                    id: "workflow-webhook-operator".into(),
                },
            )
            .map_err(Into::into)
    }

    /// Authenticate, authorize, and durably queue one webhook delivery.
    #[allow(clippy::too_many_arguments)]
    pub async fn ingest_webhook(
        &self,
        webhook_id: &str,
        delivery_id: &str,
        timestamp: &str,
        signature: &str,
        headers: BTreeMap<String, String>,
        body: &[u8],
        secret: &[u8],
    ) -> Result<WorkflowWebhookDispatch, WorkflowError> {
        let received = OffsetDateTime::now_utc();
        self.ingest_webhook_at(
            webhook_id,
            delivery_id,
            timestamp,
            signature,
            headers,
            body,
            secret,
            received,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn ingest_webhook_at(
        &self,
        webhook_id: &str,
        delivery_id: &str,
        timestamp: &str,
        signature: &str,
        headers: BTreeMap<String, String>,
        body: &[u8],
        secret: &[u8],
        received: OffsetDateTime,
    ) -> Result<WorkflowWebhookDispatch, WorkflowError> {
        if delivery_id.is_empty() || delivery_id.len() > MAX_WEBHOOK_DELIVERY_ID_BYTES {
            return Err(WorkflowError::InvalidDefinition(format!(
                "webhook delivery id must contain 1..={MAX_WEBHOOK_DELIVERY_ID_BYTES} bytes"
            )));
        }
        if delivery_id.chars().any(char::is_control) {
            return Err(WorkflowError::InvalidDefinition(
                "webhook delivery id cannot contain control characters".into(),
            ));
        }
        validate_webhook_headers(&headers)?;
        let webhook = self
            .repository
            .webhook(webhook_id)?
            .ok_or_else(|| WorkflowError::NotFound(format!("workflow webhook {webhook_id}")))?;
        if !webhook.enabled {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow webhook {webhook_id} is disabled"
            )));
        }
        let body_limit = usize::try_from(webhook.max_body_bytes)
            .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?;
        if body.is_empty() || body.len() > body_limit {
            return Err(WorkflowError::InvalidDefinition(format!(
                "webhook body must contain 1..={} bytes",
                webhook.max_body_bytes
            )));
        }
        if secret.len() < 32 {
            return Err(WorkflowError::InvalidDefinition(
                "webhook HMAC secret must contain at least 32 bytes".into(),
            ));
        }
        let signed_at = parse_schedule_time(timestamp, "webhook timestamp")?;
        let age_seconds = (received - signed_at).whole_seconds().unsigned_abs();
        if age_seconds > webhook.replay_window_seconds {
            return Err(WorkflowError::InvalidTransition(
                "webhook timestamp is outside the configured replay window".into(),
            ));
        }
        verify_webhook_signature(timestamp, delivery_id, body, signature, secret)?;
        if self
            .repository
            .webhook_delivery(webhook_id, delivery_id)?
            .is_some()
        {
            return Err(WorkflowError::InvalidTransition(format!(
                "webhook delivery {delivery_id} was already accepted"
            )));
        }
        if let Err(error) = self.validate_webhook_trust(&webhook) {
            if !matches!(&error, WorkflowError::Store(_)) {
                self.block_webhook(
                    webhook_id,
                    "pinned workflow definition or call graph is no longer trusted",
                    received,
                )?;
            }
            return Err(error);
        }
        let body_value: Value = serde_json::from_slice(body).map_err(|error| {
            WorkflowError::InvalidDefinition(format!("webhook body must be strict JSON: {error}"))
        })?;
        let inputs = json!({
            "body": body_value,
            "delivery_id": delivery_id,
            "headers": headers,
            "timestamp": timestamp,
        });
        let (definition, _) = self
            .repository
            .definition(&webhook.workflow_name, &webhook.workflow_version)?
            .ok_or_else(|| WorkflowError::NotFound(webhook.workflow_name.clone()))?;
        validate_instance(&definition.inputs, &inputs, "webhook input")?;
        let run_id = webhook_run_id(webhook_id, delivery_id);
        let body_sha256 = hex::encode(Sha256::digest(body));
        let secret_hash = hex::encode(Sha256::digest(secret));
        self.effects
            .run(WorkflowEffect {
                kind: "workflow".into(),
                action: "workflow.webhook.ingest".into(),
                content: json!({
                    "webhook_id": webhook_id,
                    "delivery_id": delivery_id,
                    "timestamp": timestamp,
                    "headers": inputs["headers"].clone(),
                    "body": inputs["body"].clone(),
                    "body_bytes": body.len(),
                    "body_sha256": body_sha256,
                    "replay_window_seconds": webhook.replay_window_seconds,
                    "workflow_name": webhook.workflow_name,
                    "workflow_version": webhook.workflow_version,
                }),
                idempotency: Some(format!("webhook:{webhook_id}:{delivery_id}")),
                credential_references: vec![CredentialReference {
                    reference: webhook.secret_reference.clone(),
                    value_hash: Some(secret_hash),
                }],
                run_id: run_id.clone(),
                step_id: "$webhook".into(),
                definition_step_id: "$webhook".into(),
                workflow_hash: webhook.workflow_hash.clone(),
                attempt: 1,
                compensation: false,
            })
            .await?;

        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        if self
            .repository
            .webhook_delivery(webhook_id, delivery_id)?
            .is_some()
        {
            return Err(WorkflowError::InvalidTransition(format!(
                "webhook delivery {delivery_id} was already accepted"
            )));
        }
        let current = self
            .repository
            .webhook(webhook_id)?
            .ok_or_else(|| WorkflowError::NotFound(format!("workflow webhook {webhook_id}")))?;
        if !current.enabled || current != webhook {
            return Err(WorkflowError::InvalidTransition(
                "webhook configuration changed during authorization; retry with current state"
                    .into(),
            ));
        }
        self.validate_webhook_trust(&current)?;
        if self.repository.run(&run_id)?.is_some() {
            return Err(WorkflowError::InvalidTransition(format!(
                "deterministic webhook run {run_id} already exists"
            )));
        }
        let received_at = format_schedule_time(received)?;
        let delivery = WorkflowWebhookDelivery {
            webhook_id: webhook_id.into(),
            delivery_id: delivery_id.into(),
            timestamp: timestamp.into(),
            received_at,
            body_sha256,
            run_id: run_id.clone(),
        };
        self.journal.append_batch(vec![
            webhook_delivery_event(&current, &delivery),
            webhook_run_event(&current, &run_id, delivery_id, inputs),
        ])?;
        Ok(WorkflowWebhookDispatch {
            delivery,
            run: self.get_run(&run_id)?,
        })
    }

    fn validate_webhook_trust(&self, webhook: &WorkflowWebhook) -> Result<(), WorkflowError> {
        let (definition, current_hash) = self
            .repository
            .definition(&webhook.workflow_name, &webhook.workflow_version)?
            .ok_or_else(|| WorkflowError::NotFound(webhook.workflow_name.clone()))?;
        if current_hash != webhook.workflow_hash {
            return Err(WorkflowError::InvalidTransition(
                "webhook pinned workflow definition changed".into(),
            ));
        }
        validate_call_graph(self.repository.as_ref(), &definition, true)
    }

    fn block_webhook(
        &self,
        webhook_id: &str,
        reason: &str,
        now: OffsetDateTime,
    ) -> Result<(), WorkflowError> {
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let Some(mut webhook) = self.repository.webhook(webhook_id)? else {
            return Ok(());
        };
        if !webhook.enabled {
            return Ok(());
        }
        webhook.enabled = false;
        webhook.blocked_reason = Some(reason.into());
        webhook.updated_at = format_schedule_time(now)?;
        let stream_id = webhook_stream(webhook_id);
        let expected_stream_version = u64::try_from(self.journal.read_stream(&stream_id)?.len())
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: EventClassification::Workflow,
            event_type: "workflow.webhook.blocked.v1".into(),
            actor: Actor {
                actor_type: ActorType::Workflow,
                id: webhook_id.into(),
            },
            context: ExecutionContext {
                correlation_id: webhook_id.into(),
                workflow_id: Some(webhook_id.into()),
                workflow_hash: Some(webhook.workflow_hash.clone()),
                ..ExecutionContext::default()
            },
            payload: json!({"record": webhook, "reason": reason}),
        })?;
        Ok(())
    }

    /// Create one bounded, hash-pinned repository-event subscription.
    #[allow(clippy::too_many_arguments)]
    pub fn create_subscription(
        &self,
        subscription_id: &str,
        workflow_name: &str,
        workflow_version: &str,
        event_type: &str,
        stream_prefix: Option<&str>,
        enabled: bool,
        after_sequence: Option<u64>,
    ) -> Result<WorkflowSubscription, WorkflowError> {
        if subscription_id.is_empty()
            || subscription_id.len() > MAX_SUBSCRIPTION_ID_BYTES
            || !valid_name(subscription_id)
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "subscription id must contain 1..={MAX_SUBSCRIPTION_ID_BYTES} lowercase letters, digits, dots, or hyphens"
            )));
        }
        if !valid_subscription_event_type(event_type) {
            return Err(WorkflowError::InvalidDefinition(format!(
                "subscription event type must be a versioned name ending in .vN, contain at most {MAX_SUBSCRIPTION_EVENT_TYPE_BYTES} lowercase letters, digits, dots, underscores, or hyphens, and cannot target workflow lifecycle events"
            )));
        }
        if let Some(prefix) = stream_prefix
            && (prefix.is_empty()
                || prefix.len() > MAX_SUBSCRIPTION_STREAM_PREFIX_BYTES
                || prefix.chars().any(char::is_control))
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "subscription stream prefix must contain 1..={MAX_SUBSCRIPTION_STREAM_PREFIX_BYTES} non-control bytes"
            )));
        }
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        if self.repository.subscription(subscription_id)?.is_some() {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow subscription {subscription_id} already exists"
            )));
        }
        if self
            .repository
            .subscriptions(MAX_WORKFLOW_SUBSCRIPTIONS)?
            .len()
            >= MAX_WORKFLOW_SUBSCRIPTIONS
        {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow subscription limit {MAX_WORKFLOW_SUBSCRIPTIONS} is exhausted"
            )));
        }
        let (definition, workflow_hash) = self
            .repository
            .definition(workflow_name, workflow_version)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("{workflow_name}:{workflow_version}"))
            })?;
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        let (head, _) = self.journal.head()?;
        let checkpoint = after_sequence.unwrap_or(head);
        if checkpoint > head {
            return Err(WorkflowError::InvalidDefinition(format!(
                "subscription checkpoint {checkpoint} is beyond journal head {head}"
            )));
        }
        let now = format_schedule_time(OffsetDateTime::now_utc())?;
        let subscription = WorkflowSubscription {
            subscription_id: subscription_id.into(),
            workflow_name: workflow_name.into(),
            workflow_version: workflow_version.into(),
            workflow_hash,
            event_type: event_type.into(),
            stream_prefix: stream_prefix.map(str::to_owned),
            enabled,
            checkpoint,
            last_event_id: None,
            last_run_id: None,
            blocked_reason: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.repository.create_subscription(
            &subscription,
            Actor {
                actor_type: ActorType::User,
                id: "workflow-subscription-registrar".into(),
            },
        )?;
        Ok(subscription)
    }

    /// Reconstruct one canonical repository-event subscription.
    pub fn get_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<WorkflowSubscription, WorkflowError> {
        self.repository
            .subscription(subscription_id)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("workflow subscription {subscription_id}"))
            })
    }

    /// List bounded subscriptions in deterministic identifier order.
    pub fn list_subscriptions(
        &self,
        limit: usize,
    ) -> Result<Vec<WorkflowSubscription>, WorkflowError> {
        self.repository
            .subscriptions(limit.min(MAX_WORKFLOW_SUBSCRIPTIONS))
            .map_err(Into::into)
    }

    /// Explicitly enable or disable one subscription after rechecking pinned trust.
    pub fn set_subscription_enabled(
        &self,
        subscription_id: &str,
        enabled: bool,
    ) -> Result<WorkflowSubscription, WorkflowError> {
        let now = format_schedule_time(OffsetDateTime::now_utc())?;
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let subscription = self
            .repository
            .subscription(subscription_id)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("workflow subscription {subscription_id}"))
            })?;
        if enabled {
            self.validate_subscription_trust(&subscription)?;
        }
        self.repository
            .set_subscription_enabled(
                subscription_id,
                enabled,
                &now,
                Actor {
                    actor_type: ActorType::User,
                    id: "workflow-subscription-operator".into(),
                },
            )
            .map_err(Into::into)
    }

    /// Evaluate persisted subscriptions against bounded canonical journal work.
    pub async fn tick_subscriptions_now(
        &self,
    ) -> Result<Vec<WorkflowSubscriptionDispatch>, WorkflowError> {
        let subscriptions = self.repository.subscriptions(MAX_WORKFLOW_SUBSCRIPTIONS)?;
        let mut dispatches = Vec::new();
        let mut queued = 0_usize;
        for subscription in subscriptions
            .into_iter()
            .filter(|subscription| subscription.enabled)
        {
            if queued >= MAX_SUBSCRIPTION_DISPATCHES_PER_TICK {
                break;
            }
            let subscription_id = subscription.subscription_id.clone();
            let checkpoint = subscription.checkpoint;
            match self.tick_subscription(subscription).await {
                Ok(Some(dispatch)) => {
                    if dispatch.status == WorkflowSubscriptionDispatchStatus::Queued {
                        queued = queued.saturating_add(1);
                    }
                    dispatches.push(dispatch);
                }
                Ok(None) => {}
                Err(WorkflowError::Effect(_) | WorkflowError::OutcomeUnknown(_)) => {
                    dispatches.push(WorkflowSubscriptionDispatch {
                        subscription_id,
                        status: WorkflowSubscriptionDispatchStatus::Deferred,
                        checkpoint,
                        source_event_id: None,
                        run_id: None,
                        reason: Some(
                            "policy-controlled dispatch did not complete; source remains pending"
                                .into(),
                        ),
                    });
                }
                Err(error) => return Err(error),
            }
        }
        Ok(dispatches)
    }

    async fn tick_subscription(
        &self,
        subscription: WorkflowSubscription,
    ) -> Result<Option<WorkflowSubscriptionDispatch>, WorkflowError> {
        let events = self.journal.read_global(
            subscription.checkpoint.saturating_add(1),
            MAX_SUBSCRIPTION_SCAN_EVENTS,
        )?;
        if events.is_empty() {
            return Ok(None);
        }
        let matching = events
            .iter()
            .find(|event| subscription_matches(&subscription, event))
            .cloned();
        let Some(source) = matching else {
            let domain_seen = events
                .iter()
                .any(|event| event.classification == EventClassification::Domain);
            if !domain_seen && events.len() < MAX_SUBSCRIPTION_SCAN_EVENTS {
                return Ok(None);
            }
            let checkpoint = events
                .last()
                .map(|event| event.global_sequence)
                .unwrap_or(subscription.checkpoint);
            return self.advance_subscription_checkpoint(&subscription, checkpoint);
        };

        if let Some(delivery) = self
            .repository
            .subscription_delivery(&subscription.subscription_id, &source.event_id)?
        {
            return self.acknowledge_duplicate_subscription(&subscription, &source, &delivery);
        }

        let inputs = self.subscription_inputs(&subscription, &source)?;
        let definition = match self.validate_subscription_trust(&subscription) {
            Ok(definition) => definition,
            Err(WorkflowError::Store(error)) => return Err(error.into()),
            Err(_) => {
                return self.block_subscription(
                    &subscription,
                    &source,
                    "pinned workflow definition or call graph is no longer trusted",
                );
            }
        };
        if let Err(error) = validate_instance(&definition.inputs, &inputs, "subscription input") {
            if let WorkflowError::Store(error) = error {
                return Err(error.into());
            }
            return self.block_subscription(
                &subscription,
                &source,
                "source event does not satisfy the pinned workflow input schema",
            );
        }
        let run_id = subscription_run_id(&subscription.subscription_id, &source.event_id);
        let dispatch = self
            .effects
            .run(WorkflowEffect {
                kind: "workflow".into(),
                action: "workflow.subscription.dispatch".into(),
                content: json!({
                    "subscription_id": subscription.subscription_id,
                    "workflow_name": subscription.workflow_name,
                    "workflow_version": subscription.workflow_version,
                    "event": inputs["event"].clone(),
                    "idempotency_key": inputs["idempotency_key"].clone(),
                }),
                idempotency: Some(format!(
                    "subscription:{}:{}",
                    subscription.subscription_id, source.event_id
                )),
                credential_references: Vec::new(),
                run_id: run_id.clone(),
                step_id: "$subscription".into(),
                definition_step_id: "$subscription".into(),
                workflow_hash: subscription.workflow_hash.clone(),
                attempt: 1,
                compensation: false,
            })
            .await;
        if let Err(error) = dispatch {
            return match error {
                WorkflowError::Effect(_) | WorkflowError::OutcomeUnknown(_) => {
                    Ok(Some(WorkflowSubscriptionDispatch {
                        subscription_id: subscription.subscription_id,
                        status: WorkflowSubscriptionDispatchStatus::Deferred,
                        checkpoint: subscription.checkpoint,
                        source_event_id: Some(source.event_id),
                        run_id: None,
                        reason: Some(
                            "policy-controlled dispatch did not complete; source remains pending"
                                .into(),
                        ),
                    }))
                }
                error => Err(error),
            };
        }

        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let mut current = self
            .repository
            .subscription(&subscription.subscription_id)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "workflow subscription {}",
                    subscription.subscription_id
                ))
            })?;
        if !current.enabled || current.checkpoint != subscription.checkpoint {
            return Ok(None);
        }
        let persisted = self
            .journal
            .read_global(source.global_sequence, 1)?
            .into_iter()
            .next()
            .filter(|event| event.event_id == source.event_id)
            .ok_or_else(|| {
                WorkflowError::InvalidTransition(
                    "subscription source event changed during authorization".into(),
                )
            })?;
        if !subscription_matches(&current, &persisted) {
            return Err(WorkflowError::InvalidTransition(
                "subscription filter changed during authorization".into(),
            ));
        }
        if let Some(delivery) = self
            .repository
            .subscription_delivery(&current.subscription_id, &persisted.event_id)?
        {
            return self.acknowledge_duplicate_subscription_locked(
                &mut current,
                &persisted,
                &delivery,
            );
        }
        let current_inputs = self.subscription_inputs(&current, &persisted)?;
        let current_definition = self.validate_subscription_trust(&current)?;
        validate_instance(
            &current_definition.inputs,
            &current_inputs,
            "subscription input",
        )?;
        if self.repository.run(&run_id)?.is_some() {
            return Err(WorkflowError::InvalidTransition(format!(
                "deterministic subscription run {run_id} already exists without its delivery receipt"
            )));
        }
        let delivered_at = format_schedule_time(OffsetDateTime::now_utc())?;
        current.checkpoint = persisted.global_sequence;
        current.last_event_id = Some(persisted.event_id.clone());
        current.last_run_id = Some(run_id.clone());
        current.blocked_reason = None;
        current.updated_at = delivered_at.clone();
        let expected_stream_version = self.subscription_version(&current.subscription_id)?;
        let delivery = WorkflowSubscriptionDelivery {
            subscription_id: current.subscription_id.clone(),
            source_event_id: persisted.event_id.clone(),
            source_global_sequence: persisted.global_sequence,
            delivered_at,
            run_id: run_id.clone(),
        };
        self.journal.append_batch(vec![
            subscription_event(
                &current,
                expected_stream_version,
                "workflow.subscription.delivered.v1",
                json!({"record": &current, "delivery": &delivery}),
            ),
            subscription_delivery_event(&current, &delivery),
            subscription_run_event(&current, &persisted.event_id, &run_id, current_inputs),
        ])?;
        Ok(Some(WorkflowSubscriptionDispatch {
            subscription_id: current.subscription_id,
            status: WorkflowSubscriptionDispatchStatus::Queued,
            checkpoint: persisted.global_sequence,
            source_event_id: Some(persisted.event_id),
            run_id: Some(run_id),
            reason: None,
        }))
    }

    fn subscription_inputs(
        &self,
        subscription: &WorkflowSubscription,
        event: &EventEnvelope,
    ) -> Result<Value, WorkflowError> {
        let payload = self.journal.decrypt_payload(event)?;
        Ok(json!({
            "subscription_id": subscription.subscription_id,
            "idempotency_key": format!(
                "subscription:{}:{}",
                subscription.subscription_id, event.event_id
            ),
            "event": {
                "event_id": event.event_id,
                "global_sequence": event.global_sequence,
                "stream_id": event.stream_id,
                "stream_version": event.stream_version,
                "classification": event.classification,
                "event_type": event.event_type,
                "actor": event.actor,
                "context": event.context,
                "occurred_at": event.occurred_at,
                "payload": payload,
            },
        }))
    }

    fn validate_subscription_trust(
        &self,
        subscription: &WorkflowSubscription,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        let (definition, current_hash) = self
            .repository
            .definition(&subscription.workflow_name, &subscription.workflow_version)?
            .ok_or_else(|| WorkflowError::NotFound(subscription.workflow_name.clone()))?;
        if current_hash != subscription.workflow_hash {
            return Err(WorkflowError::InvalidTransition(
                "subscription pinned workflow definition changed".into(),
            ));
        }
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        Ok(definition)
    }

    fn advance_subscription_checkpoint(
        &self,
        subscription: &WorkflowSubscription,
        checkpoint: u64,
    ) -> Result<Option<WorkflowSubscriptionDispatch>, WorkflowError> {
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let mut current = self
            .repository
            .subscription(&subscription.subscription_id)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "workflow subscription {}",
                    subscription.subscription_id
                ))
            })?;
        if !current.enabled || current.checkpoint != subscription.checkpoint {
            return Ok(None);
        }
        current.checkpoint = checkpoint;
        current.updated_at = format_schedule_time(OffsetDateTime::now_utc())?;
        let expected_stream_version = self.subscription_version(&current.subscription_id)?;
        self.journal.append(subscription_event(
            &current,
            expected_stream_version,
            "workflow.subscription.checkpointed.v1",
            json!({"record": &current}),
        ))?;
        Ok(Some(WorkflowSubscriptionDispatch {
            subscription_id: current.subscription_id,
            status: WorkflowSubscriptionDispatchStatus::Checkpointed,
            checkpoint,
            source_event_id: None,
            run_id: None,
            reason: None,
        }))
    }

    fn acknowledge_duplicate_subscription(
        &self,
        subscription: &WorkflowSubscription,
        source: &EventEnvelope,
        delivery: &WorkflowSubscriptionDelivery,
    ) -> Result<Option<WorkflowSubscriptionDispatch>, WorkflowError> {
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let mut current = self
            .repository
            .subscription(&subscription.subscription_id)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "workflow subscription {}",
                    subscription.subscription_id
                ))
            })?;
        if !current.enabled || current.checkpoint != subscription.checkpoint {
            return Ok(None);
        }
        self.acknowledge_duplicate_subscription_locked(&mut current, source, delivery)
    }

    fn acknowledge_duplicate_subscription_locked(
        &self,
        current: &mut WorkflowSubscription,
        source: &EventEnvelope,
        delivery: &WorkflowSubscriptionDelivery,
    ) -> Result<Option<WorkflowSubscriptionDispatch>, WorkflowError> {
        let expected_run_id = subscription_run_id(&current.subscription_id, &source.event_id);
        let run = self.repository.run(&delivery.run_id)?.ok_or_else(|| {
            StoreError::Verification(format!(
                "subscription delivery {}:{} has no queued workflow run",
                current.subscription_id, source.event_id
            ))
        })?;
        if delivery.source_global_sequence != source.global_sequence
            || delivery.run_id != expected_run_id
            || run.trigger_kind != Some(WorkflowTriggerKind::Subscription)
            || run.trigger_id.as_deref() != Some(current.subscription_id.as_str())
            || run.trigger_occurrence.as_deref() != Some(source.event_id.as_str())
        {
            return Err(StoreError::Verification(format!(
                "subscription delivery {}:{} does not match its source event and run",
                current.subscription_id, source.event_id
            ))
            .into());
        }
        current.checkpoint = source.global_sequence;
        current.last_event_id = Some(source.event_id.clone());
        current.last_run_id = Some(delivery.run_id.clone());
        current.updated_at = format_schedule_time(OffsetDateTime::now_utc())?;
        let expected_stream_version = self.subscription_version(&current.subscription_id)?;
        self.journal.append(subscription_event(
            current,
            expected_stream_version,
            "workflow.subscription.duplicate_acknowledged.v1",
            json!({"record": &current, "delivery": delivery}),
        ))?;
        Ok(Some(WorkflowSubscriptionDispatch {
            subscription_id: current.subscription_id.clone(),
            status: WorkflowSubscriptionDispatchStatus::Duplicate,
            checkpoint: source.global_sequence,
            source_event_id: Some(source.event_id.clone()),
            run_id: Some(delivery.run_id.clone()),
            reason: None,
        }))
    }

    fn block_subscription(
        &self,
        subscription: &WorkflowSubscription,
        source: &EventEnvelope,
        reason: &str,
    ) -> Result<Option<WorkflowSubscriptionDispatch>, WorkflowError> {
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let mut current = self
            .repository
            .subscription(&subscription.subscription_id)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "workflow subscription {}",
                    subscription.subscription_id
                ))
            })?;
        if !current.enabled || current.checkpoint != subscription.checkpoint {
            return Ok(None);
        }
        current.enabled = false;
        current.blocked_reason = Some(reason.into());
        current.updated_at = format_schedule_time(OffsetDateTime::now_utc())?;
        let expected_stream_version = self.subscription_version(&current.subscription_id)?;
        self.journal.append(subscription_event(
            &current,
            expected_stream_version,
            "workflow.subscription.blocked.v1",
            json!({
                "record": &current,
                "reason": reason,
                "source_event_id": source.event_id,
                "source_global_sequence": source.global_sequence,
            }),
        ))?;
        Ok(Some(WorkflowSubscriptionDispatch {
            subscription_id: current.subscription_id,
            status: WorkflowSubscriptionDispatchStatus::Blocked,
            checkpoint: current.checkpoint,
            source_event_id: Some(source.event_id.clone()),
            run_id: None,
            reason: Some(reason.into()),
        }))
    }

    fn subscription_version(&self, subscription_id: &str) -> Result<u64, StoreError> {
        u64::try_from(
            self.journal
                .read_stream(&subscription_stream(subscription_id))?
                .len(),
        )
        .map_err(|error| StoreError::Adapter(error.to_string()))
    }

    /// Evaluate every due schedule against an explicit UTC clock value.
    pub fn tick_schedules_at(
        &self,
        now: &str,
    ) -> Result<Vec<WorkflowScheduleDispatch>, WorkflowError> {
        let now = parse_schedule_time(now, "scheduler clock")?;
        self.tick_schedules(now)
    }

    /// Evaluate every due schedule using the current UTC clock.
    pub fn tick_schedules_now(&self) -> Result<Vec<WorkflowScheduleDispatch>, WorkflowError> {
        self.tick_schedules(OffsetDateTime::now_utc())
    }

    fn tick_schedules(
        &self,
        now: OffsetDateTime,
    ) -> Result<Vec<WorkflowScheduleDispatch>, WorkflowError> {
        let now_text = format_schedule_time(now)?;
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let schedules = self.repository.schedules(MAX_WORKFLOW_SCHEDULES)?;
        let mut dispatches = Vec::new();
        for mut schedule in schedules.into_iter().filter(|schedule| schedule.enabled) {
            let next_fire = parse_schedule_time(&schedule.next_fire_at, "next schedule fire")?;
            if now < next_fire {
                continue;
            }
            let elapsed_seconds = (now - next_fire).whole_seconds();
            let cadence = i64::try_from(schedule.cadence_seconds)
                .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?;
            let due_count = u64::try_from(elapsed_seconds / cadence + 1)
                .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?;
            let latest_due = add_schedule_occurrences(
                next_fire,
                schedule.cadence_seconds,
                due_count.saturating_sub(1),
            )?;
            let latest_due_text = format_schedule_time(latest_due)?;

            let definition = self
                .repository
                .definition(&schedule.workflow_name, &schedule.workflow_version)?;
            let trust_failure = match definition.as_ref() {
                None => Some("pinned workflow definition is missing"),
                Some((_, current_hash)) if current_hash != &schedule.workflow_hash => {
                    Some("pinned workflow definition hash changed")
                }
                Some((definition, _)) => {
                    match validate_call_graph(self.repository.as_ref(), definition, true) {
                        Err(WorkflowError::Store(error)) => return Err(error.into()),
                        Err(_) => Some("pinned workflow call graph is no longer valid"),
                        Ok(()) => match validate_instance(
                            &definition.inputs,
                            &schedule.inputs,
                            "schedule input",
                        ) {
                            Err(WorkflowError::Store(error)) => return Err(error.into()),
                            Err(_) => Some("pinned workflow input is no longer valid"),
                            Ok(()) => None,
                        },
                    }
                }
            };
            if let Some(reason) = trust_failure {
                schedule.enabled = false;
                schedule.blocked_reason = Some(reason.into());
                schedule.updated_at = now_text.clone();
                let expected_version = self.schedule_version(&schedule.schedule_id)?;
                self.journal.append(schedule_event(
                    &schedule,
                    expected_version,
                    "workflow.schedule.blocked.v1",
                    json!({
                        "record": &schedule,
                        "reason": reason,
                        "scheduled_at": latest_due_text,
                    }),
                ))?;
                dispatches.push(WorkflowScheduleDispatch {
                    schedule_id: schedule.schedule_id.clone(),
                    status: WorkflowScheduleDispatchStatus::Blocked,
                    scheduled_at: Some(latest_due_text),
                    next_fire_at: schedule.next_fire_at.clone(),
                    missed_occurrences: 0,
                    run_id: None,
                    reason: Some(reason.into()),
                });
                continue;
            }

            let next_fire =
                add_schedule_occurrences(next_fire, schedule.cadence_seconds, due_count)?;
            let next_fire_text = format_schedule_time(next_fire)?;
            let skip =
                due_count > 1 && schedule.misfire_policy == WorkflowScheduleMisfirePolicy::Skip;
            schedule.next_fire_at = next_fire_text.clone();
            schedule.last_scheduled_at = Some(latest_due_text.clone());
            schedule.updated_at = now_text.clone();
            schedule.blocked_reason = None;
            let expected_schedule_version = self.schedule_version(&schedule.schedule_id)?;
            if skip {
                self.journal.append(schedule_event(
                    &schedule,
                    expected_schedule_version,
                    "workflow.schedule.skipped.v1",
                    json!({
                        "record": &schedule,
                        "scheduled_at": latest_due_text,
                        "due_occurrences": due_count,
                        "missed_occurrences": due_count,
                    }),
                ))?;
                dispatches.push(WorkflowScheduleDispatch {
                    schedule_id: schedule.schedule_id.clone(),
                    status: WorkflowScheduleDispatchStatus::Skipped,
                    scheduled_at: Some(latest_due_text),
                    next_fire_at: next_fire_text,
                    missed_occurrences: due_count,
                    run_id: None,
                    reason: None,
                });
                continue;
            }

            let run_id = scheduled_run_id(&schedule.schedule_id, &latest_due_text);
            if self.repository.run(&run_id)?.is_some() {
                return Err(WorkflowError::InvalidTransition(format!(
                    "deterministic scheduled run {run_id} already exists before its schedule transition"
                )));
            }
            schedule.last_run_id = Some(run_id.clone());
            let schedule_id = schedule.schedule_id.clone();
            let run_event = scheduled_run_event(&schedule, &run_id, &latest_due_text);
            self.journal.append_batch(vec![
                schedule_event(
                    &schedule,
                    expected_schedule_version,
                    "workflow.schedule.fired.v1",
                    json!({
                        "record": &schedule,
                        "scheduled_at": latest_due_text,
                        "due_occurrences": due_count,
                        "missed_occurrences": due_count.saturating_sub(1),
                        "run_id": run_id,
                    }),
                ),
                run_event,
            ])?;
            dispatches.push(WorkflowScheduleDispatch {
                schedule_id,
                status: WorkflowScheduleDispatchStatus::Queued,
                scheduled_at: Some(latest_due_text),
                next_fire_at: next_fire_text,
                missed_occurrences: due_count.saturating_sub(1),
                run_id: Some(run_id),
                reason: None,
            });
        }
        Ok(dispatches)
    }

    fn schedule_version(&self, schedule_id: &str) -> Result<u64, StoreError> {
        u64::try_from(
            self.journal
                .read_stream(&schedule_stream(schedule_id))?
                .len(),
        )
        .map_err(|error| StoreError::Adapter(error.to_string()))
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
        let waiting_step_id = run.waiting_step_id.clone().ok_or_else(|| {
            WorkflowError::InvalidTransition("waiting step id is absent from the journal".into())
        })?;
        let waiting_execution_id = run
            .waiting_execution_id
            .clone()
            .unwrap_or_else(|| waiting_step_id.clone());
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
            json!({
                "step_id": step_id(step),
                "execution_id": waiting_execution_id,
                "input": input.clone(),
            }),
        )?;
        let is_root = definition.steps.get(root_index).is_some_and(|root| {
            step_id(root) == step_id(step) && waiting_execution_id == step_id(step)
        });
        let mut completion = json!({
            "step_id": step_id(step),
            "execution_id": waiting_execution_id,
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
            let uncertain = events
                .iter()
                .rev()
                .find(|event| event.event_type == "workflow.step.outcome_unknown.v1")
                .map(|event| self.journal.decrypt_payload(event))
                .transpose()?;
            let uncertain_retryable = uncertain
                .as_ref()
                .and_then(|payload| payload.get("retry_allowed").and_then(Value::as_bool));
            if uncertain_retryable == Some(false) {
                return Err(WorkflowError::InvalidTransition(
                    "unknown non-idempotent effect cannot be retried by resume".into(),
                ));
            }
            if let Some(execution_id) = uncertain.as_ref().and_then(|payload| {
                payload
                    .get("execution_id")
                    .or_else(|| payload.get("step_id"))
                    .and_then(Value::as_str)
            }) && let Some(linked) = self.linked_child(run_id, execution_id)?
                && self
                    .repository
                    .run(&linked.run_id)?
                    .is_some_and(|child| child.status == WorkflowStatus::Interrupted)
            {
                return Err(WorkflowError::InvalidTransition(format!(
                    "interrupted child workflow {} must be resumed before parent {run_id}",
                    linked.run_id
                )));
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
        if let Some(child_run_id) = run.waiting_child_run_id.as_deref()
            && let Ok(child) = self.get_run(child_run_id)
            && !matches!(
                child.status,
                WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
            )
        {
            self.cancel_run(child_run_id)?;
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
                let compensation =
                    started_event.event_type == "workflow.compensation.step.started.v1";
                let started = self.journal.decrypt_payload(started_event)?;
                let step_id = started
                    .get("step_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let execution_id = started
                    .get("execution_id")
                    .and_then(Value::as_str)
                    .unwrap_or(step_id);
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
                    .any(|payload| {
                        payload
                            .get("execution_id")
                            .or_else(|| payload.get("step_id"))
                            .and_then(Value::as_str)
                            == Some(execution_id)
                    });
                if completed_after {
                    self.append_run_event(
                        &run.run_id,
                        "workflow.run.interrupted.v1",
                        json!({"reason": "startup found an abandoned run after a completed step"}),
                    )?;
                    recovered.push(self.get_run(&run.run_id)?);
                    continue;
                }
                // Compensation requires its own explicit operator path. Resuming the primary
                // sequence after an uncertain compensation would execute the wrong phase, so it
                // remains fail-closed even when the compensation declares idempotency.
                let retry_allowed = !compensation
                    && self
                        .repository
                        .definition(&run.workflow_name, &run.workflow_version)?
                        .and_then(|(definition, _)| {
                            find_step(&definition.steps, step_id).map(step_retryable)
                        })
                        .unwrap_or(false);
                self.append_run_event(
                    &run.run_id,
                    "workflow.step.outcome_unknown.v1",
                    json!({
                        "phase": if compensation { "compensation" } else { "primary" },
                        "step_id": step_id,
                        "execution_id": execution_id,
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
        let mut context = json!({"inputs": inputs, "steps": {}, "executions": {}});
        for event in &events {
            if event.event_type == "workflow.step.completed.v1" {
                let payload = self.journal.decrypt_payload(event)?;
                if let (Some(step_id), Some(output)) = (
                    payload.get("step_id").and_then(Value::as_str),
                    payload.get("output").cloned(),
                ) {
                    let execution_id = payload
                        .get("execution_id")
                        .and_then(Value::as_str)
                        .unwrap_or(step_id);
                    context["executions"][execution_id] = output.clone();
                    if execution_id == step_id {
                        context["steps"][step_id] = output;
                    }
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
                    "",
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
                            "execution_id": step_id(step),
                            "output": output,
                        }),
                    )?;
                }
                Ok(StepState::Waiting {
                    step_id: waiting_step_id,
                    execution_id,
                    reason,
                    child_run_id,
                }) => {
                    self.append_run_event(
                        run_id,
                        "workflow.run.waiting.v1",
                        json!({
                            "step_id": waiting_step_id,
                            "execution_id": execution_id,
                            "reason": reason,
                            "child_run_id": child_run_id,
                        }),
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
        scope: &str,
        context: &mut Value,
        budget: Arc<AtomicU32>,
        step_budget: u32,
        semaphore: Arc<Semaphore>,
    ) -> Result<StepState, WorkflowError> {
        let execution_id = scoped_execution_id(scope, step_id(step));
        if let Some(output) = context
            .get("executions")
            .and_then(|executions| executions.get(&execution_id))
            .cloned()
        {
            return Ok(StepState::Completed(output));
        }
        if let WorkflowStep::Workflow { id, .. } = step
            && let Some(child) = self.linked_child(run_id, &execution_id)?
        {
            return self
                .observe_child_run(run_id, id, &execution_id, &child)
                .await;
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
            json!({
                "step_id": step_id(step),
                "execution_id": execution_id,
                "attempt": attempt,
            }),
        )?;
        match step {
            WorkflowStep::Emit { value, .. } => Ok(StepState::Completed(value.clone())),
            WorkflowStep::WaitForInput { id, prompt, .. } => Ok(StepState::Waiting {
                step_id: id.clone(),
                execution_id: execution_id.clone(),
                reason: prompt.clone(),
                child_run_id: None,
            }),
            WorkflowStep::Approval { id, prompt, .. } => Ok(StepState::Waiting {
                step_id: id.clone(),
                execution_id: execution_id.clone(),
                reason: prompt.clone(),
                child_run_id: None,
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
                        idempotency: idempotency
                            .as_ref()
                            .map(|strategy| format!("{strategy}:{run_id}:{execution_id}")),
                        credential_references: Vec::new(),
                        run_id: run_id.into(),
                        step_id: execution_id.clone(),
                        definition_step_id: id.clone(),
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
                        idempotency: idempotency
                            .as_ref()
                            .map(|strategy| format!("{strategy}:{run_id}:{execution_id}")),
                        credential_references: Vec::new(),
                        run_id: run_id.into(),
                        step_id: execution_id.clone(),
                        definition_step_id: id.clone(),
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
            } => {
                self.run_effect_with_retry(
                    WorkflowEffect {
                        kind: "workflow".into(),
                        action: "workflow.start".into(),
                        content: json!({
                            "workflow": workflow,
                            "version": version,
                            "inputs": inputs,
                        }),
                        idempotency: Some(format!("subworkflow:{run_id}:{execution_id}")),
                        credential_references: Vec::new(),
                        run_id: run_id.into(),
                        step_id: execution_id.clone(),
                        definition_step_id: id.clone(),
                        workflow_hash: workflow_hash.into(),
                        attempt,
                        compensation: false,
                    },
                    Arc::clone(&budget),
                    step_budget,
                )
                .await?;
                let child_run_id = Uuid::now_v7().to_string();
                let parent = self.get_run(run_id)?;
                let call_depth = parent.call_depth.saturating_add(1);
                self.append_run_event(
                    run_id,
                    "workflow.subworkflow.linked.v1",
                    json!({
                        "step_id": id,
                        "execution_id": execution_id,
                        "child_run_id": child_run_id,
                        "workflow_name": workflow,
                        "workflow_version": version,
                        "inputs": inputs,
                        "call_depth": call_depth,
                    }),
                )?;
                let child = LinkedWorkflowCall {
                    run_id: child_run_id,
                    workflow_name: workflow.clone(),
                    workflow_version: version.clone(),
                    inputs: inputs.clone(),
                    call_depth,
                };
                self.observe_child_run(run_id, id, &execution_id, &child)
                    .await
            }
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
                    scope,
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
                    &execution_id,
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
                    let iteration_scope = format!("{execution_id}[{index}]");
                    let mut iteration = context.clone();
                    iteration["item"] = item;
                    iteration["index"] = json!(index);
                    let state = self
                        .execute_sequence(
                            run_id,
                            workflow_hash,
                            steps,
                            &iteration_scope,
                            &mut iteration,
                            Arc::clone(&budget),
                            step_budget,
                            Arc::clone(&semaphore),
                        )
                        .await?;
                    if let StepState::Waiting {
                        step_id,
                        execution_id,
                        reason,
                        child_run_id,
                    } = state
                    {
                        return Ok(StepState::Waiting {
                            step_id,
                            execution_id,
                            reason,
                            child_run_id,
                        });
                    }
                    if let Some(object) = iteration.as_object_mut() {
                        object.remove("executions");
                    }
                    outputs.push(iteration);
                }
                Ok(StepState::Completed(Value::Array(outputs)))
            }
        }
    }

    fn linked_child(
        &self,
        parent_run_id: &str,
        execution_id: &str,
    ) -> Result<Option<LinkedWorkflowCall>, WorkflowError> {
        for event in self
            .journal
            .read_stream(&format!("workflow-run:{parent_run_id}"))?
            .iter()
            .rev()
            .filter(|event| event.event_type == "workflow.subworkflow.linked.v1")
        {
            let payload = self.journal.decrypt_payload(event)?;
            if payload
                .get("execution_id")
                .or_else(|| payload.get("step_id"))
                .and_then(Value::as_str)
                == Some(execution_id)
            {
                return Ok(Some(LinkedWorkflowCall {
                    run_id: string_field(&payload, "child_run_id")?,
                    workflow_name: string_field(&payload, "workflow_name")?,
                    workflow_version: string_field(&payload, "workflow_version")?,
                    inputs: payload.get("inputs").cloned().unwrap_or(Value::Null),
                    call_depth: payload
                        .get("call_depth")
                        .and_then(Value::as_u64)
                        .and_then(|depth| u16::try_from(depth).ok())
                        .ok_or_else(|| {
                            WorkflowError::InvalidTransition(
                                "linked child call depth is absent or invalid".into(),
                            )
                        })?,
                }));
            }
        }
        Ok(None)
    }

    #[async_recursion]
    async fn observe_child_run(
        &self,
        parent_run_id: &str,
        parent_step_id: &str,
        parent_execution_id: &str,
        linked: &LinkedWorkflowCall,
    ) -> Result<StepState, WorkflowError> {
        if self.repository.run(&linked.run_id)?.is_none() {
            self.queue_run_with_lineage(
                &linked.run_id,
                &linked.workflow_name,
                &linked.workflow_version,
                linked.inputs.clone(),
                Some(parent_run_id),
                Some(parent_step_id),
                Some(parent_execution_id),
                linked.call_depth,
            )?;
        }
        let mut child = self.get_run(&linked.run_id)?;
        if child.status == WorkflowStatus::Queued {
            child = self.run_queued(&linked.run_id).await?;
        }
        match child.status {
            WorkflowStatus::Completed => {
                let output = json!({
                    "run_id": child.run_id,
                    "workflow_hash": child.workflow_hash,
                    "outputs": child.outputs,
                });
                self.append_run_event(
                    parent_run_id,
                    "workflow.subworkflow.completed.v1",
                    json!({
                        "step_id": parent_step_id,
                        "execution_id": parent_execution_id,
                        "child_run_id": linked.run_id,
                        "output": output,
                    }),
                )?;
                Ok(StepState::Completed(output))
            }
            WorkflowStatus::Queued | WorkflowStatus::Running | WorkflowStatus::Waiting => {
                Ok(StepState::Waiting {
                    step_id: parent_step_id.into(),
                    execution_id: parent_execution_id.into(),
                    reason: format!("waiting for child workflow run {}", linked.run_id),
                    child_run_id: Some(linked.run_id.clone()),
                })
            }
            WorkflowStatus::Failed | WorkflowStatus::Cancelled | WorkflowStatus::Interrupted => {
                Err(WorkflowError::Effect(format!(
                    "child workflow run {} reached {}",
                    linked.run_id,
                    workflow_status_name(child.status)
                )))
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
                        "step_id": effect.definition_step_id,
                        "execution_id": effect.step_id,
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
                    json!({
                        "step_id": effect.definition_step_id,
                        "execution_id": effect.step_id,
                        "attempt": retry_attempt,
                        "retry": true,
                    }),
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
                    credential_references: Vec::new(),
                    run_id: run_id.into(),
                    step_id: id.clone(),
                    definition_step_id: id.clone(),
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
                    credential_references: Vec::new(),
                    run_id: run_id.into(),
                    step_id: id.clone(),
                    definition_step_id: id.clone(),
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
        scope: &str,
        context: &Value,
        budget: Arc<AtomicU32>,
        step_budget: u32,
        semaphore: Arc<Semaphore>,
    ) -> Result<StepState, WorkflowError> {
        let concurrency = usize::try_from(max_concurrency)
            .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?;
        let base_context = context.clone();
        let owned_branches = branches.to_vec();
        let scope = scope.to_owned();
        let results = stream::iter(owned_branches.into_iter().enumerate())
            .map(|(index, branch)| {
                let mut branch_context = base_context.clone();
                let budget = Arc::clone(&budget);
                let semaphore = Arc::clone(&semaphore);
                let branch_scope = format!("{scope}.branch[{index}]");
                async move {
                    let state = self
                        .execute_sequence(
                            run_id,
                            workflow_hash,
                            &branch,
                            &branch_scope,
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
        if let Some((step_id, execution_id, reason, child_run_id)) =
            ordered.iter().find_map(|(_, state, _)| match state {
                StepState::Waiting {
                    step_id,
                    execution_id,
                    reason,
                    child_run_id,
                } => Some((
                    step_id.clone(),
                    execution_id.clone(),
                    reason.clone(),
                    child_run_id.clone(),
                )),
                StepState::Completed(_) => None,
            })
        {
            return Ok(StepState::Waiting {
                step_id,
                execution_id,
                reason,
                child_run_id,
            });
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
        scope: &str,
        context: &mut Value,
        budget: Arc<AtomicU32>,
        step_budget: u32,
        semaphore: Arc<Semaphore>,
    ) -> Result<StepState, WorkflowError> {
        for step in steps {
            let execution_id = scoped_execution_id(scope, step_id(step));
            let already_completed = context
                .get("executions")
                .and_then(|executions| executions.get(&execution_id))
                .is_some();
            match self
                .execute_step(
                    run_id,
                    workflow_hash,
                    step,
                    scope,
                    context,
                    Arc::clone(&budget),
                    step_budget,
                    Arc::clone(&semaphore),
                )
                .await?
            {
                StepState::Completed(output) => {
                    context["executions"][&execution_id] = output.clone();
                    context["steps"][step_id(step)] = output;
                    if !already_completed {
                        self.append_run_event(
                            run_id,
                            "workflow.step.completed.v1",
                            json!({
                                "step_id": step_id(step),
                                "execution_id": execution_id,
                                "output": context["steps"][step_id(step)],
                            }),
                        )?;
                    }
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
    Waiting {
        step_id: String,
        execution_id: String,
        reason: String,
        child_run_id: Option<String>,
    },
}

struct LinkedWorkflowCall {
    run_id: String,
    workflow_name: String,
    workflow_version: String,
    inputs: Value,
    call_depth: u16,
}

fn workflow_status_name(status: WorkflowStatus) -> &'static str {
    match status {
        WorkflowStatus::Queued => "queued",
        WorkflowStatus::Running => "running",
        WorkflowStatus::Waiting => "waiting",
        WorkflowStatus::Completed => "completed",
        WorkflowStatus::Failed => "failed",
        WorkflowStatus::Cancelled => "cancelled",
        WorkflowStatus::Interrupted => "interrupted",
    }
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
mod tests;
