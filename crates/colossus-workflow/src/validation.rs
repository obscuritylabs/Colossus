use super::*;

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

pub(super) fn validate_compensation_steps(
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

pub(super) fn workflow_references(steps: &[WorkflowStep], references: &mut Vec<(String, String)>) {
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

pub(super) fn validate_call_graph(
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

pub(super) fn reject_direct_workflow_cycle(
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

pub(super) fn valid_name(name: &str) -> bool {
    name.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
    })
}

pub(super) fn validate_steps(
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

pub(super) fn step_id(step: &WorkflowStep) -> &str {
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

pub(super) fn scoped_execution_id(scope: &str, step_id: &str) -> String {
    if scope.is_empty() {
        step_id.into()
    } else {
        format!("{scope}/{step_id}")
    }
}

pub(super) fn find_step<'a>(steps: &'a [WorkflowStep], id: &str) -> Option<&'a WorkflowStep> {
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

pub(super) fn step_retryable(step: &WorkflowStep) -> bool {
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

pub(super) fn valid_step_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}
