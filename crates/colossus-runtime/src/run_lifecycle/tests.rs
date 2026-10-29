use super::*;
use crate::{
    Runtime, RuntimeBrowserHost, RuntimeComputerHost, RuntimeConfig, RuntimeOpenOptions,
    test_support::private_tempdir,
};
use colossus_contracts::*;
use colossus_ports::*;
use std::sync::atomic::{AtomicBool, Ordering};

struct Browser;
#[async_trait]
impl BrowserDriver for Browser {
    fn capabilities(&self) -> BrowserCapabilities {
        let mut capabilities = BrowserCapabilities::unavailable();
        capabilities.available = true;
        capabilities.engine_version = Some("fixture".into());
        capabilities.modes = vec![BrowserMode::Headless];
        capabilities
    }
    async fn open_session(
        &self,
        _: BrowserDriverOpenRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        Err(BrowserDriverError::Unavailable)
    }
    async fn execute(
        &self,
        _: BrowserDriverCommand,
        _: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        Err(BrowserDriverError::Unavailable)
    }
    async fn cancel_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        Ok(())
    }
    async fn close_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        Ok(())
    }
}

#[derive(Default)]
struct Computer {
    revoked: AtomicBool,
    closed: AtomicBool,
}
#[async_trait]
impl ComputerDriver for Computer {
    fn capabilities(&self) -> ComputerCapabilities {
        ComputerCapabilities {
            available: true,
            platform: "windows".into(),
            limits: Default::default(),
            unavailable_reason: None,
        }
    }
    async fn execute(
        &self,
        _: ComputerDriverRequest,
        _: &RunControl,
    ) -> Result<ComputerDriverReply, ComputerDriverError> {
        Ok(ComputerDriverReply::Windows(Vec::new()))
    }
    fn revoke_run(&self, _: &ComputerBinding) {
        self.revoked.store(true, Ordering::SeqCst);
    }
    async fn close_run(&self, _: &ComputerBinding) -> Result<(), ComputerDriverError> {
        assert!(self.revoked.load(Ordering::SeqCst));
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn combined_lifecycle_owns_every_optional_host_combination_and_fences_before_cleanup() {
    for (browser, computer) in [(false, false), (true, false), (false, true), (true, true)] {
        let workspace = private_tempdir();
        let mut config = RuntimeConfig::offline_template(workspace.path().join("state.redb"));
        config.use_ephemeral_storage();
        config.computer.enabled = computer;
        let driver = Arc::new(Computer::default());
        let mut options = RuntimeOpenOptions::for_workspace(workspace.path()).unwrap();
        if browser {
            options = options.with_browser_host(RuntimeBrowserHost::new(Arc::new(Browser)));
        }
        if computer {
            options = options.with_computer_host(RuntimeComputerHost::new(driver.clone()));
        }
        let runtime =
            Runtime::open_with_options(&config, Arc::new(DenyApproval), None, options).unwrap();
        let lifecycle = RuntimeRunLifecycle::new(
            Arc::clone(&runtime.process_sessions),
            runtime.browser.clone(),
            runtime.computer.clone(),
        );
        let context = ExecutionContext {
            run_id: Some("lifecycle-run".into()),
            session_id: Some("conversation".into()),
            ..Default::default()
        };
        lifecycle
            .begin_run(
                &context,
                &Actor {
                    actor_type: ActorType::Application,
                    id: "app:fixture".into(),
                },
                RunControl::default(),
            )
            .unwrap();
        lifecycle.tools_offered(&context).unwrap();
        assert!(runtime.process_sessions.active_run_owner(&context).is_ok());
        lifecycle.finish_run("lifecycle-run").await;
        assert!(runtime.process_sessions.active_run_owner(&context).is_err());
        assert!(!runtime.browser_active_work());
        if computer {
            assert!(!runtime.computer_active_work());
            assert!(driver.revoked.load(Ordering::SeqCst));
            assert!(driver.closed.load(Ordering::SeqCst));
        }
    }
}

#[tokio::test]
async fn native_session_lifecycle_rolls_back_failed_registration_and_preserves_duplicate_owner() {
    let workspace = private_tempdir();
    let mut config = RuntimeConfig::offline_template(workspace.path().join("state.redb"));
    config.use_ephemeral_storage();
    config.computer.enabled = true;
    let driver = Arc::new(Computer::default());
    let options = RuntimeOpenOptions::for_workspace(workspace.path())
        .unwrap()
        .with_browser_host(RuntimeBrowserHost::new(Arc::new(Browser)))
        .with_computer_host(RuntimeComputerHost::new(driver.clone()));
    let runtime =
        Runtime::open_with_options(&config, Arc::new(DenyApproval), None, options).unwrap();
    let session = runtime.create_session(None).unwrap();
    let lifecycle = RuntimeRunLifecycle::new(
        Arc::clone(&runtime.process_sessions),
        runtime.browser.clone(),
        runtime.computer.clone(),
    )
    .with_session_lifecycle(Arc::clone(&runtime.native_session_lifecycle));
    let context = ExecutionContext {
        run_id: Some("registration".into()),
        session_id: Some(session.id.clone()),
        ..Default::default()
    };
    let control = RunControl::default();
    let invalid = Actor {
        actor_type: ActorType::Application,
        id: String::new(),
    };
    assert!(
        lifecycle
            .begin_run(&context, &invalid, control.clone())
            .is_err()
    );
    assert!(!control.is_cancelled());
    assert!(runtime.process_sessions.active_run_owner(&context).is_err());
    assert!(
        !runtime
            .browser
            .as_ref()
            .unwrap()
            .has_run_registration("registration")
    );
    runtime.set_session_archived(&session.id, true).unwrap();
    runtime.set_session_archived(&session.id, false).unwrap();
    let owner = Actor {
        actor_type: ActorType::Application,
        id: "app:fixture".into(),
    };
    lifecycle
        .begin_run(&context, &owner, control.clone())
        .unwrap();
    assert!(
        lifecycle
            .begin_run(&context, &owner, RunControl::default())
            .is_err()
    );
    assert!(!control.is_cancelled());
    assert_eq!(
        runtime.process_sessions.active_run_owner(&context).unwrap(),
        owner
    );
    assert!(runtime.set_session_archived(&session.id, true).is_err());
    lifecycle.finish_run("registration").await;
    runtime.set_session_archived(&session.id, true).unwrap();
}
