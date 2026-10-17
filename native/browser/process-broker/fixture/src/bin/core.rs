#[cfg(feature = "core-entry")]
fn main() {
    let mut stage = "entry";
    if let Err(error) = run(&mut stage) {
        eprintln!(
            "EVIDENCE supervisor-core stage={stage} kind={:?}",
            error.kind()
        );
        std::process::exit(20)
    }
}
#[cfg(not(feature = "core-entry"))]
fn main() {
    std::process::exit(78)
}
#[cfg(feature = "core-entry")]
fn run(stage: &mut &'static str) -> std::io::Result<()> {
    use colossus_darwin_process::take_connected_parent_channel;
    use colossus_mac_browser_process_broker::CoreProcessOwner;
    use colossus_mac_supervisor_sentinel_fixture::{
        CONTROLLER,
        inventory::{Inventory, invalid},
        native, wire,
    };
    use std::{
        io::{Read as _, Write as _},
        net::Shutdown,
        time::Duration,
    };
    if std::env::args_os().count() != 1 {
        return Err(invalid("Core fixture accepts no arguments"));
    }
    *stage = "controller_admission";
    let (mut controller, parent) = take_connected_parent_channel(CONTROLLER)?.into_parts();
    controller.set_read_timeout(Some(Duration::from_secs(30)))?;
    controller.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mode = wire::read_exact::<1>(&mut controller)?[0];
    if !(1..=4).contains(&mode) {
        return Err(invalid("invalid fixed supervisor case"));
    }
    *stage = "inventory_admission";
    let inventory = Inventory::load()?;
    if inventory.controller_cdhash != CONTROLLER {
        return Err(invalid("independent controller seal changed"));
    }
    parent.verify()?;
    let mut owner = CoreProcessOwner::new(inventory.launch())?;
    *stage = "supervisor_start";
    let mut channels = owner.start()?;
    channels
        .bootstrap
        .set_read_timeout(Some(Duration::from_secs(20)))?;
    *stage = "sentinel_ready";
    let mut ready = [0; 12];
    if let Err(error) = channels.bootstrap.read_exact(&mut ready) {
        // Explicit cleanup preserves the independent creator receipt and
        // actual direct supervisor statuses before this startup error escapes.
        let closed = owner.close();
        let statuses = owner.fixture_supervisor_statuses();
        eprintln!(
            "EVIDENCE supervisor-startup-close close_ok={} creator_status={:?} keeper_status={:?}",
            closed.is_ok(),
            statuses[0],
            statuses[1]
        );
        if let Some(receipt) = owner.receipt() {
            eprintln!(
                "EVIDENCE supervisor-startup-roster main_status={:?} helpers_reaped={} unknown_creations={} keeper_empty={} output_drained={} channels_closed={}",
                receipt.main_status,
                receipt.helpers_reaped,
                receipt.unknown_creations,
                receipt.keeper_empty,
                receipt.output_drained,
                receipt.channels_closed
            );
        }
        for output in owner.fixture_supervisor_output()? {
            emit_native_categories(&output);
        }
        return Err(error);
    }
    if ready != *b"CSENT001\x01\x02\x03\x04" {
        return Err(invalid("sentinel role readiness missing"));
    }
    parent.verify()?;
    *stage = "controller_ready";
    controller.write_all(wire::READY)?;
    if matches!(mode, 3 | 4) {
        *stage = "crash_observation_transfer";
        let observed = owner.crash_fixture_observation()?;
        controller.write_all(&observed.creator_hint.to_be_bytes())?;
        controller.write_all(&observed.keeper_hint.to_be_bytes())?;
        native::send_receipt(&controller, &observed.receipt_reader)?;
        if mode == 3 {
            for reader in &observed.diagnostic_readers {
                native::send_receipt(&controller, reader)?;
            }
        }
        *stage = "crash_observer_admission";
        if wire::read_exact::<8>(&mut controller)? != *b"CSACK001" {
            return Err(invalid("external observers not admitted"));
        }
        parent.verify()?;
        controller.write_all(b"CEXIT073")?;
        // No owner/pipe/control Drop executes. The coordinator is the sole active
        // receipt reader only after this actual abrupt process exit.
        let _retained = (owner, channels, controller, observed);
        std::process::exit(73);
    }
    *stage = "case_admission";
    if wire::read_exact::<8>(&mut controller)? != *b"CSACK001" {
        return Err(invalid("fixture case not admitted"));
    }
    *stage = "core_eof";
    if mode == 2 {
        channels.data.shutdown(Shutdown::Both)?;
    }
    *stage = "supervisor_cleanup";
    let closed = owner.close();
    let receipt = owner
        .receipt()
        .ok_or_else(|| invalid("creator receipt unavailable"))?;
    let statuses = owner.fixture_supervisor_statuses();
    let value =
        serde_json::json!({"receipt":receipt,"statuses":statuses,"close_ok":closed.is_ok()});
    *stage = "controller_result";
    wire::frame(
        &mut controller,
        &serde_json::to_vec(&value).map_err(std::io::Error::other)?,
    )?;
    closed
}

#[cfg(feature = "core-entry")]
fn emit_native_categories(output: &[u8]) {
    let text = String::from_utf8_lossy(output);
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("EVIDENCE native-process-entry mode=") {
            let Some((mode, kind)) = value.split_once(" kind=") else {
                continue;
            };
            let mode = match mode {
                "creator" => "creator",
                "keeper" => "keeper",
                _ => continue,
            };
            let kind = match kind {
                "UnexpectedEof" | "InvalidData" | "PermissionDenied" | "ConnectionReset"
                | "BrokenPipe" | "TimedOut" | "NotFound" | "Other" | "Unsupported"
                | "InvalidInput" | "Interrupted" | "WouldBlock" | "AlreadyExists"
                | "NotConnected" => kind,
                _ => "unclassified",
            };
            eprintln!("EVIDENCE supervisor-native mode={mode} kind={kind}");
        } else if let Some(value) = line.strip_prefix("EVIDENCE native-process-stage phase=") {
            let Some((phase, reason)) = value.split_once(" reason=") else {
                continue;
            };
            let phase = match phase {
                "mode_admission"
                | "keeper_admission"
                | "keeper_setup"
                | "keeper_parent_recheck"
                | "keeper_service"
                | "creator_admission"
                | "creator_setup"
                | "creator_parent_recheck"
                | "creator_keeper_recheck"
                | "creator_watchers"
                | "creator_owner"
                | "main_spawn"
                | "main_output"
                | "main_relays"
                | "main_serve"
                | "creator_cleanup"
                | "creator_receipt" => phase,
                _ => "unclassified",
            };
            let reason = match reason {
                "keeper_scope" | "keeper_parent" | "keeper_identity" | "keeper_stream"
                | "keeper_credentials" | "self_code_hash" | "peer_parent" | "peer_incarnation"
                | "peer_identity" | "peer_credentials" | "peer_stream" | "peer_code"
                | "peer_guest" | "peer_changed" | "setup_incomplete" | "setup_header"
                | "setup_images" | "setup_environment" | "marker_namespace" | "marker_policy"
                | "marker_generation" => reason,
                _ => "unclassified",
            };
            eprintln!("EVIDENCE supervisor-native-stage phase={phase} reason={reason}");
        } else if let Some(value) = line.strip_prefix("EVIDENCE browser-helper-exit role=") {
            let Some((role, status)) = value.split_once(" raw_status=") else {
                continue;
            };
            if let (Ok(role @ 1..=4), Ok(status @ 0..=65535)) =
                (role.parse::<u8>(), status.parse::<u32>())
            {
                eprintln!("EVIDENCE supervisor-helper-exit role={role} raw_status={status}");
            }
        }
    }
}
