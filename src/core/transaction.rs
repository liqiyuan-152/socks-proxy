use crate::{
    routing::RoutingMode,
    storage::{AppConfig, ConfigError, ConfigStore},
};
use std::fmt;

const RESTART_WARNING: &str = "切换需要重启网络内核，现有 SSH 等连接可能中断";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchPhase {
    Direct,
    Running,
    Reconfiguring,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationSnapshot {
    pub phase: SwitchPhase,
    pub desired_mode: RoutingMode,
    pub applied_mode: Option<RoutingMode>,
    pub applied_revision: u64,
    pub failure: Option<String>,
    pub traffic_may_be_direct: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyPlan {
    Hot,
    Restart,
}

pub trait ApplicationRuntime {
    type Error: std::error::Error + Send + Sync + 'static;

    fn plan(
        &self,
        current: &AppConfig,
        current_mode: RoutingMode,
        candidate: &AppConfig,
        candidate_mode: RoutingMode,
    ) -> ApplyPlan;
    fn validate(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error>;
    fn apply(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error>;
    fn rollback(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error>;
    fn shutdown(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn cache_initialized(&self) -> Option<bool> {
        None
    }
    fn connection_events(&self) -> Vec<crate::logs::ConnectionEvent> {
        Vec::new()
    }
    fn clear_connection_events(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn poll_failure(&mut self) -> Result<Option<String>, String> {
        Ok(None)
    }
}

pub trait ApplicationStore {
    type Error: std::error::Error + Send + Sync + 'static;

    fn save_applied(&mut self, config: &AppConfig) -> Result<(), Self::Error>;
}

impl ApplicationStore for ConfigStore {
    type Error = ConfigError;

    fn save_applied(&mut self, config: &AppConfig) -> Result<(), Self::Error> {
        self.save(config)
    }
}

pub struct SwitchRequest {
    pub config: AppConfig,
    pub mode: RoutingMode,
    pub restart_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchResult {
    pub applied_config: AppConfig,
    pub plan: ApplyPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchError {
    Invalid(String),
    RestartConfirmationRequired(&'static str),
    Runtime(String),
    Persistence(String),
    Rollback { cause: String, rollback: String },
}

impl fmt::Display for SwitchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => write!(formatter, "候选配置无效: {error}"),
            Self::RestartConfirmationRequired(warning) => formatter.write_str(warning),
            Self::Runtime(error) => write!(formatter, "应用运行配置失败: {error}"),
            Self::Persistence(error) => write!(formatter, "保存已应用配置失败: {error}"),
            Self::Rollback { cause, rollback } => {
                write!(formatter, "切换失败 ({cause})，回滚也失败 ({rollback})")
            }
        }
    }
}

impl std::error::Error for SwitchError {}

pub struct ApplicationController<R, S> {
    current: AppConfig,
    runtime: R,
    store: S,
    snapshot: ApplicationSnapshot,
}

impl<R, S> ApplicationController<R, S>
where
    R: ApplicationRuntime,
    S: ApplicationStore,
{
    pub fn new(current: AppConfig, runtime: R, store: S) -> Result<Self, SwitchError> {
        current
            .validate()
            .map_err(|error| SwitchError::Invalid(error.to_string()))?;
        let mode = current.last_applied_mode;
        Ok(Self {
            snapshot: ApplicationSnapshot {
                phase: phase_for(mode),
                desired_mode: mode,
                applied_mode: Some(mode),
                applied_revision: current.revision,
                failure: None,
                traffic_may_be_direct: false,
            },
            current,
            runtime,
            store,
        })
    }

    pub fn new_restoring(current: AppConfig, runtime: R, store: S) -> Result<Self, SwitchError> {
        current
            .validate()
            .map_err(|error| SwitchError::Invalid(error.to_string()))?;
        let desired_mode = current.last_applied_mode;
        let mut controller = Self {
            snapshot: ApplicationSnapshot {
                phase: SwitchPhase::Direct,
                desired_mode,
                applied_mode: Some(RoutingMode::Direct),
                applied_revision: current.revision,
                failure: None,
                traffic_may_be_direct: false,
            },
            current,
            runtime,
            store,
        };
        let _ = controller.retry_last_successful();
        Ok(controller)
    }

    pub fn snapshot(&self) -> &ApplicationSnapshot {
        &self.snapshot
    }

    pub fn current_config(&self) -> &AppConfig {
        &self.current
    }

    pub fn connection_events(&self) -> Vec<crate::logs::ConnectionEvent> {
        self.runtime.connection_events()
    }

    pub fn clear_connection_events(&mut self) -> Result<(), String> {
        self.runtime.clear_connection_events()
    }

    pub fn refresh_runtime(&mut self) {
        if self.snapshot.phase != SwitchPhase::Running {
            return;
        }
        match self.runtime.poll_failure() {
            Ok(Some(detail)) => self.observe_runtime_exit(detail),
            Ok(None) => {}
            Err(error) => self.observe_runtime_exit(format!("内核监管失败: {error}")),
        }
    }

    pub fn observe_runtime_exit(&mut self, detail: impl Into<String>) {
        self.snapshot.phase = SwitchPhase::Error;
        self.snapshot.applied_mode = None;
        self.snapshot.failure = Some(detail.into());
        self.snapshot.traffic_may_be_direct = true;
    }

    pub fn shutdown(&mut self) -> Result<(), SwitchError> {
        match self.runtime.shutdown() {
            Ok(()) => {
                self.snapshot.phase = SwitchPhase::Direct;
                self.snapshot.applied_mode = Some(RoutingMode::Direct);
                self.snapshot.failure = None;
                self.snapshot.traffic_may_be_direct = false;
                Ok(())
            }
            Err(error) => {
                let failure = SwitchError::Runtime(error.to_string());
                self.snapshot.phase = SwitchPhase::Error;
                self.snapshot.applied_mode = None;
                self.snapshot.failure = Some(failure.to_string());
                self.snapshot.traffic_may_be_direct = true;
                Err(failure)
            }
        }
    }

    pub fn retry_last_successful(&mut self) -> Result<(), SwitchError> {
        let mode = self.current.last_applied_mode;
        self.snapshot.desired_mode = mode;
        self.snapshot.failure = None;
        if mode != RoutingMode::Direct && self.current.profiles.active().is_none() {
            self.snapshot.phase = SwitchPhase::Direct;
            self.snapshot.applied_mode = Some(RoutingMode::Direct);
            self.snapshot.failure = Some("需要先配置当前代理".into());
            self.snapshot.traffic_may_be_direct = false;
            return Ok(());
        }
        if let Err(error) = self.runtime.validate(&self.current, mode) {
            let failure = SwitchError::Invalid(error.to_string());
            self.snapshot.phase = SwitchPhase::Error;
            self.snapshot.applied_mode = Some(RoutingMode::Direct);
            self.snapshot.failure = Some(failure.to_string());
            self.snapshot.traffic_may_be_direct = true;
            return Err(failure);
        }
        if let Err(error) = self.runtime.apply(&self.current, mode) {
            let failure = SwitchError::Runtime(error.to_string());
            self.snapshot.phase = SwitchPhase::Error;
            self.snapshot.applied_mode = None;
            self.snapshot.failure = Some(failure.to_string());
            self.snapshot.traffic_may_be_direct = true;
            return Err(failure);
        }
        self.snapshot.phase = phase_for(mode);
        self.snapshot.applied_mode = Some(mode);
        self.snapshot.failure = None;
        self.snapshot.traffic_may_be_direct = false;
        Ok(())
    }

    pub fn switch(&mut self, request: SwitchRequest) -> Result<SwitchResult, SwitchError> {
        let mut candidate = request.config;
        candidate
            .validate()
            .map_err(|error| SwitchError::Invalid(error.to_string()))?;
        if request.mode != RoutingMode::Direct {
            candidate
                .profiles
                .require_active()
                .map_err(|error| SwitchError::Invalid(error.to_string()))?;
        }
        let old = self.current.clone();
        let old_mode = self.snapshot.applied_mode.unwrap_or(RoutingMode::Direct);
        candidate.revision = old.revision.saturating_add(1);
        candidate.last_applied_mode = request.mode;
        let plan = self.runtime.plan(&old, old_mode, &candidate, request.mode);
        if plan == ApplyPlan::Restart && !request.restart_confirmed {
            return Err(SwitchError::RestartConfirmationRequired(RESTART_WARNING));
        }
        self.runtime
            .validate(&candidate, request.mode)
            .map_err(|error| SwitchError::Invalid(error.to_string()))?;

        self.snapshot.phase = SwitchPhase::Reconfiguring;
        self.snapshot.desired_mode = request.mode;
        self.snapshot.failure = None;
        if let Err(error) = self.runtime.apply(&candidate, request.mode) {
            return self.rollback_after_failure(
                &old,
                old_mode,
                SwitchError::Runtime(error.to_string()),
            );
        }
        if let Some(cache_initialized) = self.runtime.cache_initialized() {
            candidate.cache_initialized = cache_initialized;
        }
        if let Err(error) = self.store.save_applied(&candidate) {
            return self.rollback_after_failure(
                &old,
                old_mode,
                SwitchError::Persistence(error.to_string()),
            );
        }

        self.current = candidate.clone();
        self.snapshot = ApplicationSnapshot {
            phase: phase_for(request.mode),
            desired_mode: request.mode,
            applied_mode: Some(request.mode),
            applied_revision: candidate.revision,
            failure: None,
            traffic_may_be_direct: false,
        };
        Ok(SwitchResult {
            applied_config: candidate,
            plan,
        })
    }

    pub fn stage_without_runtime(
        &mut self,
        mut candidate: AppConfig,
        desired_mode: RoutingMode,
    ) -> Result<(), SwitchError> {
        candidate
            .validate()
            .map_err(|error| SwitchError::Invalid(error.to_string()))?;
        candidate.revision = self.current.revision.saturating_add(1);
        candidate.last_applied_mode = desired_mode;
        self.store
            .save_applied(&candidate)
            .map_err(|error| SwitchError::Persistence(error.to_string()))?;
        self.current = candidate.clone();
        self.snapshot = ApplicationSnapshot {
            phase: SwitchPhase::Direct,
            desired_mode,
            applied_mode: Some(RoutingMode::Direct),
            applied_revision: candidate.revision,
            failure: Some("需要先配置当前代理".into()),
            traffic_may_be_direct: false,
        };
        Ok(())
    }

    fn rollback_after_failure<T>(
        &mut self,
        old: &AppConfig,
        old_mode: RoutingMode,
        cause: SwitchError,
    ) -> Result<T, SwitchError> {
        let cause_text = cause.to_string();
        match self.runtime.rollback(old, old_mode) {
            Ok(()) => {
                self.snapshot = ApplicationSnapshot {
                    phase: phase_for(old_mode),
                    desired_mode: old_mode,
                    applied_mode: Some(old_mode),
                    applied_revision: old.revision,
                    failure: Some(cause_text),
                    traffic_may_be_direct: false,
                };
                Err(cause)
            }
            Err(error) => {
                let failure = SwitchError::Rollback {
                    cause: cause_text,
                    rollback: error.to_string(),
                };
                self.snapshot.phase = SwitchPhase::Error;
                self.snapshot.applied_mode = None;
                self.snapshot.failure = Some(failure.to_string());
                self.snapshot.traffic_may_be_direct = true;
                Err(failure)
            }
        }
    }
}

fn phase_for(mode: RoutingMode) -> SwitchPhase {
    if mode == RoutingMode::Direct {
        SwitchPhase::Direct
    } else {
        SwitchPhase::Running
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ProfileId, ProxyHost, ProxyProfile, ProxyProfiles, ProxyProtocol};
    use std::io;

    #[derive(Default)]
    struct Runtime {
        plan: Option<ApplyPlan>,
        fail_validate: bool,
        fail_apply: bool,
        fail_rollback: bool,
        fail_shutdown: bool,
        events: Vec<(String, RoutingMode, Option<String>)>,
    }

    impl ApplicationRuntime for Runtime {
        type Error = io::Error;

        fn plan(
            &self,
            _current: &AppConfig,
            _current_mode: RoutingMode,
            _candidate: &AppConfig,
            _candidate_mode: RoutingMode,
        ) -> ApplyPlan {
            self.plan.unwrap_or(ApplyPlan::Hot)
        }

        fn validate(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error> {
            self.events.push(("validate".into(), mode, active(config)));
            if self.fail_validate {
                Err(io::Error::other("invalid core config"))
            } else {
                Ok(())
            }
        }

        fn apply(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error> {
            self.events.push(("apply".into(), mode, active(config)));
            if self.fail_apply {
                Err(io::Error::other("apply failed"))
            } else {
                Ok(())
            }
        }

        fn rollback(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error> {
            self.events.push(("rollback".into(), mode, active(config)));
            if self.fail_rollback {
                Err(io::Error::other("rollback failed"))
            } else {
                Ok(())
            }
        }

        fn shutdown(&mut self) -> Result<(), Self::Error> {
            self.events
                .push(("shutdown".into(), RoutingMode::Direct, None));
            if self.fail_shutdown {
                Err(io::Error::other("dns flush failed"))
            } else {
                Ok(())
            }
        }
    }

    #[derive(Default)]
    struct Store {
        fail: bool,
        saved: Vec<AppConfig>,
    }

    impl ApplicationStore for Store {
        type Error = io::Error;

        fn save_applied(&mut self, config: &AppConfig) -> Result<(), Self::Error> {
            if self.fail {
                Err(io::Error::other("disk full"))
            } else {
                self.saved.push(config.clone());
                Ok(())
            }
        }
    }

    fn profile(name: &str, port: u16) -> ProxyProfile {
        ProxyProfile {
            id: ProfileId::new(),
            name: name.into(),
            protocol: ProxyProtocol::Socks5,
            host: ProxyHost::parse("127.0.0.1").unwrap(),
            port,
            auth_enabled: false,
            credential_ref: None,
        }
    }

    fn config() -> (AppConfig, ProfileId, ProfileId) {
        let a = profile("A", 1080);
        let b = profile("B", 1081);
        let (a_id, b_id) = (a.id.clone(), b.id.clone());
        let mut profiles = ProxyProfiles::default();
        profiles.create(a).unwrap();
        profiles.create(b).unwrap();
        profiles.select(&a_id).unwrap();
        (
            AppConfig {
                revision: 8,
                profiles,
                last_applied_mode: RoutingMode::Rules,
                ..AppConfig::default()
            },
            a_id,
            b_id,
        )
    }

    fn active(config: &AppConfig) -> Option<String> {
        config.profiles.active().map(|profile| profile.name.clone())
    }

    #[test]
    fn switching_proxy_and_mode_commits_only_after_runtime_success() {
        let (current, _, b) = config();
        let mut candidate = current.clone();
        candidate.profiles.select(&b).unwrap();
        let mut controller =
            ApplicationController::new(current, Runtime::default(), Store::default()).unwrap();
        let result = controller
            .switch(SwitchRequest {
                config: candidate,
                mode: RoutingMode::GlobalProxy,
                restart_confirmed: false,
            })
            .unwrap();
        assert_eq!(result.applied_config.revision, 9);
        assert_eq!(active(&result.applied_config).as_deref(), Some("B"));
        assert_eq!(controller.snapshot().phase, SwitchPhase::Running);
        assert_eq!(
            controller.snapshot().applied_mode,
            Some(RoutingMode::GlobalProxy)
        );
        assert_eq!(controller.runtime.events[0].0, "validate");
        assert_eq!(controller.runtime.events[1].0, "apply");
        assert_eq!(controller.store.saved.len(), 1);
    }

    #[test]
    fn restart_plan_requires_disruption_confirmation_before_changes() {
        let (current, _, _) = config();
        let runtime = Runtime {
            plan: Some(ApplyPlan::Restart),
            ..Runtime::default()
        };
        let mut controller =
            ApplicationController::new(current.clone(), runtime, Store::default()).unwrap();
        let error = controller
            .switch(SwitchRequest {
                config: current,
                mode: RoutingMode::GlobalProxy,
                restart_confirmed: false,
            })
            .unwrap_err();
        assert!(matches!(error, SwitchError::RestartConfirmationRequired(_)));
        assert!(controller.runtime.events.is_empty());
        assert_eq!(controller.snapshot().phase, SwitchPhase::Running);
    }

    #[test]
    fn validation_failure_never_changes_runtime_or_snapshot() {
        let (current, _, _) = config();
        let runtime = Runtime {
            fail_validate: true,
            ..Runtime::default()
        };
        let mut controller =
            ApplicationController::new(current.clone(), runtime, Store::default()).unwrap();
        assert!(matches!(
            controller.switch(SwitchRequest {
                config: current,
                mode: RoutingMode::GlobalProxy,
                restart_confirmed: false,
            }),
            Err(SwitchError::Invalid(_))
        ));
        assert_eq!(controller.runtime.events.len(), 1);
        assert_eq!(controller.snapshot().applied_revision, 8);
    }

    #[test]
    fn apply_or_persistence_failure_restores_old_proxy_and_mode() {
        for (runtime, store, expected) in [
            (
                Runtime {
                    fail_apply: true,
                    ..Runtime::default()
                },
                Store::default(),
                "应用运行配置失败",
            ),
            (
                Runtime::default(),
                Store {
                    fail: true,
                    ..Store::default()
                },
                "保存已应用配置失败",
            ),
        ] {
            let (current, _, b) = config();
            let mut candidate = current.clone();
            candidate.profiles.select(&b).unwrap();
            let mut controller = ApplicationController::new(current, runtime, store).unwrap();
            let error = controller
                .switch(SwitchRequest {
                    config: candidate,
                    mode: RoutingMode::GlobalProxy,
                    restart_confirmed: false,
                })
                .unwrap_err();
            assert!(error.to_string().contains(expected));
            assert_eq!(active(controller.current_config()).as_deref(), Some("A"));
            assert_eq!(controller.snapshot().applied_mode, Some(RoutingMode::Rules));
            assert_eq!(controller.runtime.events.last().unwrap().0, "rollback");
        }
    }

    #[test]
    fn failed_rollback_enters_error_without_claiming_an_applied_mode() {
        let (current, _, _) = config();
        let runtime = Runtime {
            fail_apply: true,
            fail_rollback: true,
            ..Runtime::default()
        };
        let mut controller =
            ApplicationController::new(current.clone(), runtime, Store::default()).unwrap();
        assert!(matches!(
            controller.switch(SwitchRequest {
                config: current,
                mode: RoutingMode::GlobalProxy,
                restart_confirmed: false,
            }),
            Err(SwitchError::Rollback { .. })
        ));
        assert_eq!(controller.snapshot().phase, SwitchPhase::Error);
        assert_eq!(controller.snapshot().applied_mode, None);
        assert!(controller.snapshot().traffic_may_be_direct);
    }

    #[test]
    fn unexpected_exit_preserves_last_successful_mode_and_supports_retry() {
        let (current, _, _) = config();
        let mut controller =
            ApplicationController::new(current, Runtime::default(), Store::default()).unwrap();
        controller.observe_runtime_exit("内核意外退出，流量可能直连");
        assert_eq!(controller.snapshot().phase, SwitchPhase::Error);
        assert_eq!(controller.snapshot().desired_mode, RoutingMode::Rules);
        assert_eq!(controller.snapshot().applied_mode, None);
        assert!(controller.snapshot().traffic_may_be_direct);
        assert_eq!(
            controller.current_config().last_applied_mode,
            RoutingMode::Rules
        );
        assert_eq!(controller.store.saved.len(), 0);

        controller.retry_last_successful().unwrap();
        assert_eq!(controller.snapshot().phase, SwitchPhase::Running);
        assert_eq!(controller.snapshot().applied_mode, Some(RoutingMode::Rules));
        assert!(!controller.snapshot().traffic_may_be_direct);
        assert_eq!(controller.store.saved.len(), 0);
    }

    #[test]
    fn shutdown_restores_direct_without_overwriting_last_successful_mode() {
        let (current, _, _) = config();
        let mut controller =
            ApplicationController::new(current, Runtime::default(), Store::default()).unwrap();

        controller.shutdown().unwrap();

        assert_eq!(controller.snapshot().phase, SwitchPhase::Direct);
        assert_eq!(
            controller.snapshot().applied_mode,
            Some(RoutingMode::Direct)
        );
        assert_eq!(
            controller.current_config().last_applied_mode,
            RoutingMode::Rules
        );
        assert_eq!(controller.store.saved.len(), 0);
        assert_eq!(controller.runtime.events.last().unwrap().0, "shutdown");
    }

    #[test]
    fn failed_shutdown_stays_visible_and_does_not_exit_cleanly() {
        let (current, _, _) = config();
        let runtime = Runtime {
            fail_shutdown: true,
            ..Runtime::default()
        };
        let mut controller =
            ApplicationController::new(current, runtime, Store::default()).unwrap();

        let error = controller.shutdown().unwrap_err();

        assert!(error.to_string().contains("dns flush failed"));
        assert_eq!(controller.snapshot().phase, SwitchPhase::Error);
        assert_eq!(controller.snapshot().applied_mode, None);
        assert!(controller.snapshot().traffic_may_be_direct);
        assert_eq!(
            controller.current_config().last_applied_mode,
            RoutingMode::Rules
        );
    }

    #[test]
    fn startup_restore_failure_never_claims_the_persisted_proxy_mode() {
        let (current, _, _) = config();
        let runtime = Runtime {
            fail_validate: true,
            ..Runtime::default()
        };
        let controller =
            ApplicationController::new_restoring(current, runtime, Store::default()).unwrap();
        assert_eq!(controller.snapshot().phase, SwitchPhase::Error);
        assert_eq!(controller.snapshot().desired_mode, RoutingMode::Rules);
        assert_eq!(
            controller.snapshot().applied_mode,
            Some(RoutingMode::Direct)
        );
        assert!(controller.snapshot().failure.is_some());
        assert!(controller.snapshot().traffic_may_be_direct);
    }

    #[test]
    fn startup_with_rules_but_no_proxy_stays_direct_without_touching_runtime() {
        let current = AppConfig::default();
        assert_eq!(current.last_applied_mode, RoutingMode::Rules);
        let controller =
            ApplicationController::new_restoring(current, Runtime::default(), Store::default())
                .unwrap();

        assert_eq!(controller.snapshot().desired_mode, RoutingMode::Rules);
        assert_eq!(controller.snapshot().phase, SwitchPhase::Direct);
        assert_eq!(
            controller.snapshot().applied_mode,
            Some(RoutingMode::Direct)
        );
        assert_eq!(
            controller.snapshot().failure.as_deref(),
            Some("需要先配置当前代理")
        );
        assert!(!controller.snapshot().traffic_may_be_direct);
        assert!(controller.runtime.events.is_empty());
    }
}
