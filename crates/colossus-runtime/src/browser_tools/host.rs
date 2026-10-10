use colossus_browser_bridge::{BrowserHostFactory, BrowserHostPool};
use colossus_ports::BrowserDriver;
use std::{fmt, sync::Arc};

/// Native-composition browser driver binding. It carries no model-facing authority.
///
/// Construct this only after verifying the component, private transport, engine
/// sandbox, and egress containment. Drivers remain unavailable unless they report
/// those capabilities. No executable path or certificate material enters runtime YAML.
#[derive(Clone)]
pub struct RuntimeBrowserHost {
    pub(super) driver: Arc<dyn BrowserDriver>,
    pub(super) presenter: Option<Arc<dyn super::RuntimeBrowserPresenter>>,
    profile_roots: Vec<colossus_home::ConfinedRoot>,
}

impl RuntimeBrowserHost {
    /// Bind a driver selected and verified by the trusted runtime host.
    #[must_use]
    pub fn new(driver: Arc<dyn BrowserDriver>) -> Self {
        Self {
            driver,
            presenter: None,
            profile_roots: Vec::new(),
        }
    }

    /// Bind a private presentation provider from the same verified native supervisor.
    /// This does not establish accepted Embedded capability or human authority.
    #[must_use]
    pub fn with_presenter(mut self, presenter: Arc<dyn super::RuntimeBrowserPresenter>) -> Self {
        self.presenter = Some(presenter);
        self
    }

    /// Protect the same native profile store independently of model policy/grants.
    /// Roots must come from verified native supervision, never renderer/model paths.
    #[must_use]
    pub fn with_native_profile_protection(
        mut self,
        roots: Vec<colossus_home::ConfinedRoot>,
    ) -> Self {
        self.profile_roots = roots;
        self
    }

    pub(crate) fn protected_profile_roots(&self) -> &[colossus_home::ConfinedRoot] {
        &self.profile_roots
    }

    /// Bind a trusted factory to cancellation-safe, per-session process ownership.
    /// Every allocation gets its own immutable scope and destination enrollment.
    pub fn from_supervised_factory(
        factory: Arc<dyn BrowserHostFactory>,
    ) -> Result<Self, colossus_ports::BrowserDriverError> {
        Ok(Self::new(Arc::new(BrowserHostPool::new(factory)?)))
    }

    /// Compose the separate macOS host for explicit native acceptance only.
    ///
    /// The caller constructs and retains the exact platform supervisor; this seam does
    /// not discover an executable, promote a package, or acknowledge native cleanup.
    /// Requiring one object to implement both factory and presenter prevents Core and
    /// Desktop from being accidentally wired to different native ownership domains.
    #[cfg(all(
        target_os = "macos",
        debug_assertions,
        feature = "macos-browser-host-acceptance"
    ))]
    pub fn macos_supervised_for_acceptance<F>(
        supervisor: Arc<F>,
    ) -> Result<(Self, Arc<F>), colossus_ports::BrowserDriverError>
    where
        F: BrowserHostFactory + super::RuntimeBrowserPresenter + 'static,
    {
        let factory: Arc<dyn BrowserHostFactory> = supervisor.clone();
        let presenter: Arc<dyn super::RuntimeBrowserPresenter> = supervisor.clone();
        let host = Self::from_supervised_factory(factory)?.with_presenter(presenter);
        Ok((host, supervisor))
    }

    /// Verify an installed Linux OCI browser and compose it with the shared runtime tools.
    /// The inventory digest and image identity must come from trusted native release
    /// verification. This does not discover a personal browser or accept runtime YAML.
    /// The caller must retain the returned supervisor and await `shutdown` before exit;
    /// dropping a runtime binding does not acknowledge native process cleanup.
    #[cfg(target_os = "linux")]
    pub async fn linux_oci(
        config: colossus_sandbox::OciBrowserConfig,
    ) -> Result<
        (Self, Arc<colossus_sandbox::OciBrowserSupervisor>),
        colossus_ports::BrowserDriverError,
    > {
        let factory = Arc::new(colossus_sandbox::OciBrowserSupervisor::install(config).await?);
        Self::bind_linux_oci(factory).await
    }

    /// Explicit debug acceptance composition; never use for ordinary installation discovery.
    /// Its unsigned fixture inventory cannot establish production browser support.
    /// Retain the returned supervisor and await its shutdown independently of runtime errors.
    #[cfg(all(target_os = "linux", debug_assertions))]
    pub async fn linux_oci_for_acceptance(
        config: colossus_sandbox::OciBrowserConfig,
    ) -> Result<
        (Self, Arc<colossus_sandbox::OciBrowserSupervisor>),
        colossus_ports::BrowserDriverError,
    > {
        let factory =
            Arc::new(colossus_sandbox::OciBrowserSupervisor::install_for_acceptance(config).await?);
        Self::bind_linux_oci(factory).await
    }

    #[cfg(target_os = "linux")]
    async fn bind_linux_oci(
        factory: Arc<colossus_sandbox::OciBrowserSupervisor>,
    ) -> Result<
        (Self, Arc<colossus_sandbox::OciBrowserSupervisor>),
        colossus_ports::BrowserDriverError,
    > {
        match Self::from_supervised_factory(factory.clone()) {
            Ok(host) => {
                let roots = factory.protected_profile_roots();
                Ok((
                    host.with_presenter(factory.clone())
                        .with_native_profile_protection(roots),
                    factory,
                ))
            }
            Err(error) => {
                factory.shutdown().await?;
                Err(error)
            }
        }
    }

    /// Compose an independently publisher-verified Windows AppContainer/Job/WFP host.
    /// Retain the returned supervisor and await its shutdown independently of runtime
    /// errors. Native containment acceptance must precede ordinary discovery.
    #[cfg(windows)]
    pub async fn windows_supervised(
        config: colossus_sandbox::WindowsBrowserConfig,
    ) -> Result<
        (Self, Arc<colossus_sandbox::WindowsBrowserSupervisor>),
        colossus_ports::BrowserDriverError,
    > {
        let factory = Arc::new(colossus_sandbox::WindowsBrowserSupervisor::install(config).await?);
        Self::bind_windows_supervised(factory).await
    }

    /// Explicit debug native acceptance composition; unavailable to release discovery.
    /// Retain the returned native cleanup owner even when browser enrollment fails.
    #[cfg(all(windows, debug_assertions))]
    pub async fn windows_supervised_for_acceptance(
        config: colossus_sandbox::WindowsBrowserConfig,
    ) -> Result<
        (Self, Arc<colossus_sandbox::WindowsBrowserSupervisor>),
        colossus_ports::BrowserDriverError,
    > {
        let factory = Arc::new(
            colossus_sandbox::WindowsBrowserSupervisor::install_for_acceptance(config).await?,
        );
        Self::bind_windows_supervised(factory).await
    }

    #[cfg(windows)]
    async fn bind_windows_supervised(
        factory: Arc<colossus_sandbox::WindowsBrowserSupervisor>,
    ) -> Result<
        (Self, Arc<colossus_sandbox::WindowsBrowserSupervisor>),
        colossus_ports::BrowserDriverError,
    > {
        match Self::from_supervised_factory(factory.clone()) {
            Ok(host) => Ok((host.with_presenter(factory.clone()), factory)),
            Err(error) => {
                factory.shutdown().await?;
                Err(error)
            }
        }
    }
}

impl fmt::Debug for RuntimeBrowserHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeBrowserHost")
            .finish_non_exhaustive()
    }
}

#[cfg(all(
    test,
    target_os = "macos",
    debug_assertions,
    feature = "macos-browser-host-acceptance"
))]
mod macos_acceptance_tests {
    use super::*;
    use async_trait::async_trait;
    use colossus_browser_presentation::PresentationClient;
    use colossus_contracts::BrowserCapabilities;
    use colossus_ports::{BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest};

    struct AcceptanceSupervisor;

    #[async_trait]
    impl BrowserHostFactory for AcceptanceSupervisor {
        fn capabilities(&self) -> BrowserCapabilities {
            BrowserCapabilities::unavailable()
        }

        async fn launch(
            &self,
            _: &BrowserDriverOpenRequest,
            _: &BrowserDriverControl,
        ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
            Err(BrowserDriverError::Unavailable)
        }

        async fn reap_failed_launch(
            &self,
            _: &BrowserDriverOpenRequest,
        ) -> Result<(), BrowserDriverError> {
            Ok(())
        }
    }

    #[async_trait]
    impl super::super::RuntimeBrowserPresenter for AcceptanceSupervisor {
        async fn acquire(
            &self,
            _: &BrowserDriverOpenRequest,
        ) -> Result<(PresentationClient, [u8; 32]), BrowserDriverError> {
            Err(BrowserDriverError::Unavailable)
        }
    }

    #[test]
    fn macos_acceptance_composition_retains_one_factory_and_presenter_owner() {
        let supervisor = Arc::new(AcceptanceSupervisor);
        let (host, retained) =
            RuntimeBrowserHost::macos_supervised_for_acceptance(supervisor.clone()).unwrap();

        assert!(host.presenter.is_some());
        assert!(Arc::ptr_eq(&supervisor, &retained));
        assert!(Arc::strong_count(&supervisor) >= 4);
    }
}
