use super::Failure;
use std::{fs::OpenOptions, io::Write as _, os::unix::fs::OpenOptionsExt as _, path::Path};

#[derive(Default)]
pub struct Evidence {
    pub accepted: bool,
    pub normal_tls: bool,
    pub leaf_pin_matches: bool,
    pub upstream_observed: bool,
    pub proxy_credential_stripped: bool,
    pub shutdown_ack: bool,
    pub listener_v4_closed: bool,
    pub listener_v6_closed: bool,
    pub tracked_socket_closed: bool,
    pub broker_exit_observed: bool,
    pub broker_reaped: bool,
    pub parent_reaped: bool,
    pub parent_crashed_without_drop: bool,
    pub forced: bool,
    pub broker_exit_code: Option<i32>,
    pub parent_exit_code: Option<i32>,
    pub stdout_empty: bool,
    pub category: &'static str,
    pub cleanup_category: &'static str,
}

impl Evidence {
    pub fn write(&self, directory: &Path) -> Result<(), Failure> {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.join("result.json"))
            .map_err(|_| "fixture_evidence_unavailable")?;
        let code =
            |value: Option<i32>| value.map_or_else(|| "null".into(), |value| value.to_string());
        writeln!(output, "{{\n  \"accepted\": {},\n  \"normal_tls\": {},\n  \"leaf_pin_matches\": {},\n  \"upstream_observed\": {},\n  \"proxy_credential_stripped\": {},\n  \"shutdown_ack\": {},\n  \"listener_v4_closed\": {},\n  \"listener_v6_closed\": {},\n  \"tracked_socket_closed\": {},\n  \"broker_exit_observed\": {},\n  \"broker_reaped\": {},\n  \"parent_reaped\": {},\n  \"parent_crashed_without_drop\": {},\n  \"forced\": {},\n  \"broker_exit_code\": {},\n  \"parent_exit_code\": {},\n  \"stdout_empty\": {},\n  \"category\": \"{}\",\n  \"cleanup_category\": \"{}\",\n  \"full_helper_cleanup_proven\": false,\n  \"production_enabled\": false\n}}", self.accepted,self.normal_tls,self.leaf_pin_matches,self.upstream_observed,self.proxy_credential_stripped,self.shutdown_ack,self.listener_v4_closed,self.listener_v6_closed,self.tracked_socket_closed,self.broker_exit_observed,self.broker_reaped,self.parent_reaped,self.parent_crashed_without_drop,self.forced,code(self.broker_exit_code),code(self.parent_exit_code),self.stdout_empty,self.category,self.cleanup_category).map_err(|_| "fixture_evidence_unavailable")?;
        output
            .sync_all()
            .map_err(|_| "fixture_evidence_unavailable")
    }
}
