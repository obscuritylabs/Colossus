//! Effect-free finite durable owner transitions and conservative recovery points.
use crate::{MutationOperation, Status};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub device: u64,
    pub inode: u64,
    pub uid: u32,
    pub mode: u32,
    pub birth_seconds: u64,
    pub birth_nanos: u32,
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Header {
    pub schema_version: u32,
    pub purpose: String,
    pub package_binding_sha256: String,
    pub broker_cdhash: String,
    pub parent_cdhash: String,
    pub parent_generation_binding_sha256: String,
    pub generation_sha256: String,
    pub broker_pid: u32,
    pub broker_generation_binding_sha256: String,
    pub parent_directory: Binding,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    Namespace,
    Create,
    Import,
    Retire,
}
impl From<MutationOperation> for Operation {
    fn from(value: MutationOperation) -> Self {
        match value {
            MutationOperation::CreateNamespace => Self::Namespace,
            MutationOperation::CreateStore => Self::Create,
            MutationOperation::ImportIdentity => Self::Import,
            MutationOperation::RetireStore => Self::Retire,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Output {
    pub kind: u32,
    pub binding: Binding,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Event {
    Pending {
        sequence: u64,
        operation: Operation,
    },
    Sealed {
        sequence: u64,
        outputs: Vec<Output>,
    },
    Retirement {
        sequence: u64,
        step: u32,
        suffix: String,
    },
    UnchangedForRetirement {
        sequence: u64,
        outputs: Vec<Output>,
    },
}
#[derive(Clone)]
pub(super) struct State {
    sequence: u64,
    pending: Option<Operation>,
    namespace: Option<Output>,
    store: Option<(Output, Output)>,
    retirement: Option<(u32, String)>,
    finished: bool,
    quarantined: bool,
}
impl State {
    pub fn replay(expected: &Header, bytes: &[u8]) -> Result<Self, Status> {
        if bytes.is_empty() || bytes.len() > 32 * 1024 || bytes.last() != Some(&b'\n') {
            return Err(Status::OutcomeUnknown);
        }
        let mut lines = bytes.split(|byte| *byte == b'\n');
        let header: Header = serde_json::from_slice(lines.next().ok_or(Status::OutcomeUnknown)?)
            .map_err(|_| Status::OutcomeUnknown)?;
        if header != *expected {
            return Err(Status::Denied);
        }
        let mut state = Self::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let event: Event = serde_json::from_slice(line).map_err(|_| Status::OutcomeUnknown)?;
            state = state.transition(&event)?;
        }
        Ok(state)
    }
    pub fn new() -> Self {
        Self {
            sequence: 0,
            pending: None,
            namespace: None,
            store: None,
            retirement: None,
            finished: false,
            quarantined: false,
        }
    }
    pub fn transition(&self, event: &Event) -> Result<Self, Status> {
        let mut next = self.clone();
        match event {
            Event::Pending {
                sequence,
                operation,
            } => {
                if self.finished
                    || self.quarantined && *operation != Operation::Retire
                    || *sequence <= self.sequence
                    || self.pending.is_some_and(|prior| {
                        prior != Operation::Retire || *operation != Operation::Retire
                    })
                    || match operation {
                        Operation::Namespace => self.namespace.is_some(),
                        Operation::Create => self.namespace.is_none() || self.store.is_some(),
                        Operation::Import => self.store.is_none() || self.retirement.is_some(),
                        Operation::Retire => self.namespace.is_none(),
                    }
                {
                    return Err(Status::OutcomeUnknown);
                }
                next.sequence = *sequence;
                next.pending = Some(*operation);
            }
            Event::Sealed { sequence, outputs } => {
                if *sequence != self.sequence {
                    return Err(Status::OutcomeUnknown);
                }
                match self.pending {
                    Some(Operation::Namespace)
                        if outputs.len() == 1
                            && outputs[0].kind == 1
                            && outputs[0].binding.mode == 0o700 =>
                    {
                        next.namespace = Some(outputs[0].clone())
                    }
                    Some(Operation::Create | Operation::Import)
                        if outputs.len() == 2
                            && matches!(outputs[0].kind, 2 | 3)
                            && outputs[1].kind == 4
                            && outputs.iter().all(|out| out.binding.mode == 0o600) =>
                    {
                        if self
                            .store
                            .as_ref()
                            .is_some_and(|(_, lock)| lock.binding != outputs[1].binding)
                        {
                            return Err(Status::OutcomeUnknown);
                        }
                        next.store = Some((outputs[0].clone(), outputs[1].clone()));
                    }
                    _ => return Err(Status::OutcomeUnknown),
                }
                next.pending = None;
            }
            Event::Retirement {
                sequence,
                step,
                suffix,
            } => {
                if *sequence != self.sequence
                    || self.pending != Some(Operation::Retire)
                    || *step > 3
                    || suffix.len() != 32
                    || !suffix
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    || self.retirement.as_ref().is_some_and(|(old, name)| {
                        suffix != name || *step < *old || *step > *old + 1
                    })
                    || self.retirement.is_none() && *step != 0
                {
                    return Err(Status::OutcomeUnknown);
                }
                next.retirement = Some((*step, suffix.clone()));
                next.finished = *step == 3;
                if next.finished {
                    next.pending = None;
                }
            }
            Event::UnchangedForRetirement { sequence, outputs } => {
                if *sequence != self.sequence
                    || self.pending != Some(Operation::Import)
                    || outputs.len() != 2
                {
                    return Err(Status::OutcomeUnknown);
                }
                let (store, lock) = self.store.as_ref().ok_or(Status::OutcomeUnknown)?;
                if store.kind != outputs[0].kind
                    || lock.kind != outputs[1].kind
                    || store.binding != outputs[0].binding
                    || lock.binding != outputs[1].binding
                {
                    return Err(Status::OutcomeUnknown);
                }
                next.pending = None;
                next.quarantined = true;
            }
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output(kind: u32, inode: u64) -> Output {
        Output {
            kind,
            binding: Binding {
                device: 1,
                inode,
                uid: 502,
                mode: if kind == 1 { 0o700 } else { 0o600 },
                birth_seconds: 10,
                birth_nanos: 20,
            },
        }
    }
    fn owned() -> State {
        let mut state = State::new();
        for event in [
            Event::Pending {
                sequence: 1,
                operation: Operation::Namespace,
            },
            Event::Sealed {
                sequence: 1,
                outputs: vec![output(1, 1)],
            },
            Event::Pending {
                sequence: 2,
                operation: Operation::Create,
            },
            Event::Sealed {
                sequence: 2,
                outputs: vec![output(2, 2), output(4, 3)],
            },
        ] {
            state = state
                .transition(&event)
                .expect("positive physical receipts");
        }
        state
    }
    #[test]
    fn pending_effect_has_no_recovery_adoption_and_import_replacement_needs_same_lock() {
        let pending = owned()
            .transition(&Event::Pending {
                sequence: 3,
                operation: Operation::Import,
            })
            .expect("durable pending");
        assert!(
            pending
                .transition(&Event::Pending {
                    sequence: 4,
                    operation: Operation::Retire
                })
                .is_err()
        );
        assert!(
            pending
                .transition(&Event::Sealed {
                    sequence: 3,
                    outputs: vec![output(2, 4), output(4, 5)]
                })
                .is_err()
        );
        let sealed = pending
            .transition(&Event::Sealed {
                sequence: 3,
                outputs: vec![output(2, 4), output(4, 3)],
            })
            .expect("positive native replacement");
        assert!(
            sealed
                .transition(&Event::Pending {
                    sequence: 4,
                    operation: Operation::Retire
                })
                .is_ok()
        );
    }
    #[test]
    fn failed_import_retires_only_same_already_owned_generations_and_never_reuses_contents() {
        let pending = owned()
            .transition(&Event::Pending {
                sequence: 3,
                operation: Operation::Import,
            })
            .expect("pending import");
        for outputs in [
            vec![output(2, 4), output(4, 3)],
            vec![output(2, 2), output(4, 4)],
            vec![output(2, 2)],
            vec![output(3, 2), output(4, 3)],
        ] {
            assert!(
                pending
                    .transition(&Event::UnchangedForRetirement {
                        sequence: 3,
                        outputs
                    })
                    .is_err()
            );
        }
        let known = pending
            .transition(&Event::UnchangedForRetirement {
                sequence: 3,
                outputs: vec![output(2, 2), output(4, 3)],
            })
            .expect("same physical generations");
        assert!(known.quarantined);
        assert!(
            known
                .transition(&Event::Pending {
                    sequence: 4,
                    operation: Operation::Import
                })
                .is_err()
        );
        assert!(
            known
                .transition(&Event::Pending {
                    sequence: 4,
                    operation: Operation::Retire
                })
                .is_ok()
        );
    }
    #[test]
    fn retirement_cannot_skip_steps_change_namespace_or_reopen_after_completion() {
        let mut state = owned()
            .transition(&Event::Pending {
                sequence: 3,
                operation: Operation::Retire,
            })
            .expect("retire owned");
        let suffix = "a".repeat(32);
        assert!(
            state
                .transition(&Event::Retirement {
                    sequence: 3,
                    step: 1,
                    suffix: suffix.clone()
                })
                .is_err()
        );
        for step in 0..=3 {
            state = state
                .transition(&Event::Retirement {
                    sequence: 3,
                    step,
                    suffix: suffix.clone(),
                })
                .expect("ordered exact physical progress");
        }
        assert!(state.finished);
        assert!(
            state
                .transition(&Event::Pending {
                    sequence: 4,
                    operation: Operation::Retire
                })
                .is_err()
        );
    }
    #[test]
    fn replay_rejects_missing_truncated_rebound_and_untyped_records() {
        let header = Header {
            schema_version: 1,
            purpose: "colossus-macos-owned-custody-generation".into(),
            package_binding_sha256: "1".repeat(64),
            broker_cdhash: "2".repeat(40),
            parent_cdhash: "3".repeat(40),
            parent_generation_binding_sha256: "4".repeat(64),
            generation_sha256: "5".repeat(64),
            broker_pid: 100,
            broker_generation_binding_sha256: "7".repeat(64),
            parent_directory: output(1, 1).binding,
        };
        let mut bytes = serde_json::to_vec(&header).expect("header");
        bytes.push(b'\n');
        assert!(State::replay(&header, &bytes).is_ok());
        assert!(State::replay(&header, &[]).is_err());
        assert!(State::replay(&header, &bytes[..bytes.len() - 1]).is_err());
        let mut changed = header.clone();
        changed.broker_cdhash = "6".repeat(40);
        assert!(State::replay(&changed, &bytes).is_err());
        bytes.extend_from_slice(b"{\"event\":\"pending\",\"sequence\":1,\"operation\":\"sign\"}\n");
        assert!(State::replay(&header, &bytes).is_err());
    }
}
