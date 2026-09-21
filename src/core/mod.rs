use crate::routing::RoutingMode;
use std::{fmt, path::Path};

mod config;
mod process;
mod recovery;
mod runtime;
mod transaction;
pub use config::{
    CompiledCoreConfig, CoreConfigError, CoreConfigInput, CoreConfigValidator, DirectDnsServer,
    ProxyCredentials, RestrictedConfigFile,
};
pub use process::{ProcessCoreBackend, ProcessCoreError};
pub use recovery::{
    NetworkChange, NetworkRecovery, NetworkRecoveryBackend, NetworkRecoveryError,
    NetworkRecoveryReport, NetworkResource, RecoveryStore,
};
pub use runtime::{
    CoreCredentialSource, ManagedCoreRuntime, ManagedCoreRuntimeError, NoCredentials,
};
pub use transaction::{
    ApplicationController, ApplicationRuntime, ApplicationSnapshot, ApplicationStore, ApplyPlan,
    SwitchError, SwitchPhase, SwitchRequest, SwitchResult,
};

pub const PINNED_CORE_VERSION: &str = "1.14.1-socks-proxy.2";
pub const STRICT_CACHE_EXIT_CODE: i32 = 78;
pub const STRICT_CACHE_MARKER: &str = "STRICT_CACHE_ERROR";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreStatus {
    Direct,
    Starting,
    Running,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreFailureKind {
    MissingBinary,
    VersionMismatch,
    MissingInitializedCache,
    Startup,
    Readiness,
    StrictCache,
    UnexpectedExit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreFailure {
    pub kind: CoreFailureKind,
    pub message: String,
}

impl fmt::Display for CoreFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CoreFailure {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSnapshot {
    pub status: CoreStatus,
    pub desired_mode: RoutingMode,
    pub applied_mode: Option<RoutingMode>,
    pub failure: Option<CoreFailure>,
    pub traffic_may_be_direct: bool,
    pub cache_initialized: bool,
}

impl Default for CoreSnapshot {
    fn default() -> Self {
        Self {
            status: CoreStatus::Direct,
            desired_mode: RoutingMode::Direct,
            applied_mode: Some(RoutingMode::Direct),
            failure: None,
            traffic_may_be_direct: false,
            cache_initialized: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreHandle(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendStage {
    Version,
    Start,
    Readiness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorePoll {
    Running,
    Exited,
}

#[derive(Debug)]
pub struct BackendError {
    pub stage: BackendStage,
    pub missing_binary: bool,
    pub message: String,
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BackendError {}

pub trait CoreBackend {
    fn version(&mut self) -> Result<String, BackendError>;
    fn start(&mut self) -> Result<CoreHandle, BackendError>;
    fn wait_ready(&mut self, handle: CoreHandle) -> Result<(), BackendError>;
}

pub trait CoreProcessMonitor {
    type Error: std::error::Error + Send + Sync + 'static;

    fn try_wait(&mut self) -> Result<Option<i32>, Self::Error>;
    fn captured_output(&self) -> String;
}

#[derive(Debug, Default)]
pub struct CoreSupervisor {
    snapshot: CoreSnapshot,
    handle: Option<CoreHandle>,
}

impl CoreSupervisor {
    pub fn snapshot(&self) -> &CoreSnapshot {
        &self.snapshot
    }

    pub fn start<B: CoreBackend>(
        &mut self,
        mode: RoutingMode,
        cache_was_initialized: bool,
        cache_path: &Path,
        backend: &mut B,
    ) -> Result<CoreHandle, CoreFailure> {
        self.snapshot = CoreSnapshot {
            status: CoreStatus::Starting,
            desired_mode: mode,
            applied_mode: self.snapshot.applied_mode,
            failure: None,
            traffic_may_be_direct: false,
            cache_initialized: cache_was_initialized,
        };

        if cache_was_initialized && !cache_path.is_file() {
            return self.fail(
                CoreFailureKind::MissingInitializedCache,
                "持久 FakeIP 缓存曾初始化但文件已丢失",
            );
        }
        let version = backend.version().map_err(|error| {
            let kind = if error.missing_binary {
                CoreFailureKind::MissingBinary
            } else {
                CoreFailureKind::Startup
            };
            self.record_failure(kind, error.to_string())
        })?;
        if version.trim() != PINNED_CORE_VERSION {
            return self.fail(
                CoreFailureKind::VersionMismatch,
                format!("内核版本不匹配: 需要 {PINNED_CORE_VERSION}，实际 {version}"),
            );
        }
        let handle = backend
            .start()
            .map_err(|error| self.record_failure(CoreFailureKind::Startup, error.to_string()))?;
        backend
            .wait_ready(handle)
            .map_err(|error| self.record_failure(CoreFailureKind::Readiness, error.to_string()))?;
        if !cache_path.is_file() {
            return self.fail(
                CoreFailureKind::Readiness,
                "内核已报告就绪，但持久缓存文件不存在",
            );
        }
        self.handle = Some(handle);
        self.snapshot.status = CoreStatus::Running;
        self.snapshot.applied_mode = Some(mode);
        self.snapshot.cache_initialized = true;
        Ok(handle)
    }

    pub fn observe_exit(&mut self, exit_code: Option<i32>, output: &str) {
        self.handle = None;
        let strict =
            exit_code == Some(STRICT_CACHE_EXIT_CODE) || output.contains(STRICT_CACHE_MARKER);
        let failure = if strict {
            CoreFailure {
                kind: CoreFailureKind::StrictCache,
                message: "严格缓存校验失败，旧映射已保留".into(),
            }
        } else {
            CoreFailure {
                kind: CoreFailureKind::UnexpectedExit,
                message: format!("内核意外退出，退出码: {exit_code:?}"),
            }
        };
        self.snapshot.status = CoreStatus::Error;
        self.snapshot.applied_mode = None;
        self.snapshot.failure = Some(failure);
        self.snapshot.traffic_may_be_direct = true;
    }

    pub fn poll<M: CoreProcessMonitor>(
        &mut self,
        monitor: &mut M,
    ) -> Result<CorePoll, CoreFailure> {
        if self.snapshot.status != CoreStatus::Running || self.handle.is_none() {
            return self.fail(CoreFailureKind::UnexpectedExit, "没有可监管的运行内核");
        }
        match monitor.try_wait() {
            Ok(None) => Ok(CorePoll::Running),
            Ok(Some(exit_code)) => {
                self.observe_exit(Some(exit_code), &monitor.captured_output());
                Ok(CorePoll::Exited)
            }
            Err(error) => {
                self.handle = None;
                self.snapshot.applied_mode = None;
                self.snapshot.traffic_may_be_direct = true;
                self.fail(
                    CoreFailureKind::UnexpectedExit,
                    format!("监管内核进程失败: {error}"),
                )
            }
        }
    }

    pub fn retry<B: CoreBackend>(
        &mut self,
        cache_path: &Path,
        backend: &mut B,
    ) -> Result<CoreHandle, CoreFailure> {
        let mode = self.snapshot.desired_mode;
        if mode == RoutingMode::Direct {
            return self.fail(CoreFailureKind::Startup, "全局直连不启动网络内核");
        }
        self.start(mode, self.snapshot.cache_initialized, cache_path, backend)
    }

    pub fn mark_direct_recovered(&mut self) {
        self.handle = None;
        self.snapshot.status = CoreStatus::Direct;
        self.snapshot.desired_mode = RoutingMode::Direct;
        self.snapshot.applied_mode = Some(RoutingMode::Direct);
        self.snapshot.failure = None;
        self.snapshot.traffic_may_be_direct = false;
    }

    fn fail<T>(
        &mut self,
        kind: CoreFailureKind,
        message: impl Into<String>,
    ) -> Result<T, CoreFailure> {
        Err(self.record_failure(kind, message.into()))
    }

    fn record_failure(&mut self, kind: CoreFailureKind, message: String) -> CoreFailure {
        let failure = CoreFailure { kind, message };
        self.snapshot.status = CoreStatus::Error;
        self.snapshot.failure = Some(failure.clone());
        failure
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};
    use uuid::Uuid;

    struct Backend {
        version: Result<String, BackendError>,
        start: Result<CoreHandle, BackendError>,
        ready: Result<(), BackendError>,
    }

    impl CoreBackend for Backend {
        fn version(&mut self) -> Result<String, BackendError> {
            self.version.as_ref().map(Clone::clone).map_err(clone_error)
        }
        fn start(&mut self) -> Result<CoreHandle, BackendError> {
            self.start.as_ref().copied().map_err(clone_error)
        }
        fn wait_ready(&mut self, _handle: CoreHandle) -> Result<(), BackendError> {
            self.ready.as_ref().copied().map_err(clone_error)
        }
    }

    fn clone_error(error: &BackendError) -> BackendError {
        BackendError {
            stage: error.stage,
            missing_binary: error.missing_binary,
            message: error.message.clone(),
        }
    }

    fn good_backend() -> Backend {
        Backend {
            version: Ok(PINNED_CORE_VERSION.into()),
            start: Ok(CoreHandle(42)),
            ready: Ok(()),
        }
    }

    fn absent_cache() -> PathBuf {
        std::env::temp_dir().join(format!("missing-cache-{}", Uuid::new_v4()))
    }

    #[test]
    fn running_is_reported_only_after_readiness() {
        let cache = absent_cache();
        fs::write(&cache, b"cache").unwrap();
        let mut supervisor = CoreSupervisor::default();
        let mut backend = good_backend();
        supervisor
            .start(RoutingMode::Rules, false, &cache, &mut backend)
            .unwrap();
        assert_eq!(supervisor.snapshot().status, CoreStatus::Running);
        assert_eq!(supervisor.snapshot().applied_mode, Some(RoutingMode::Rules));
        assert!(supervisor.snapshot().cache_initialized);
        let _ = fs::remove_file(cache);
    }

    #[test]
    fn initialized_but_missing_cache_is_rejected_before_process_start() {
        let mut supervisor = CoreSupervisor::default();
        let mut backend = good_backend();
        let error = supervisor
            .start(RoutingMode::Rules, true, &absent_cache(), &mut backend)
            .unwrap_err();
        assert_eq!(error.kind, CoreFailureKind::MissingInitializedCache);
        assert_eq!(supervisor.snapshot().status, CoreStatus::Error);
    }

    #[test]
    fn missing_wrong_and_not_ready_never_report_running() {
        let cases = [
            Backend {
                version: Err(BackendError {
                    stage: BackendStage::Version,
                    missing_binary: true,
                    message: "missing".into(),
                }),
                start: Ok(CoreHandle(1)),
                ready: Ok(()),
            },
            Backend {
                version: Ok("wrong".into()),
                ..good_backend()
            },
            Backend {
                ready: Err(BackendError {
                    stage: BackendStage::Readiness,
                    missing_binary: false,
                    message: "not ready".into(),
                }),
                ..good_backend()
            },
        ];
        let cache = absent_cache();
        fs::write(&cache, b"cache").unwrap();
        for mut backend in cases {
            let mut supervisor = CoreSupervisor::default();
            assert!(
                supervisor
                    .start(RoutingMode::GlobalProxy, false, &cache, &mut backend)
                    .is_err()
            );
            assert_ne!(supervisor.snapshot().status, CoreStatus::Running);
            assert_ne!(
                supervisor.snapshot().applied_mode,
                Some(RoutingMode::GlobalProxy)
            );
        }
        let _ = fs::remove_file(cache);
    }

    #[test]
    fn strict_marker_or_exit_code_has_a_dedicated_failure() {
        for (code, output) in [
            (Some(78), ""),
            (Some(1), "prefix STRICT_CACHE_ERROR detail"),
        ] {
            let mut supervisor = CoreSupervisor::default();
            supervisor.observe_exit(code, output);
            assert_eq!(
                supervisor.snapshot().failure.as_ref().unwrap().kind,
                CoreFailureKind::StrictCache
            );
            assert!(supervisor.snapshot().traffic_may_be_direct);
        }
    }

    #[test]
    fn process_exit_can_retry_the_preserved_mode_or_recover_direct() {
        struct Monitor(Option<i32>);
        impl CoreProcessMonitor for Monitor {
            type Error = std::io::Error;

            fn try_wait(&mut self) -> Result<Option<i32>, Self::Error> {
                Ok(self.0)
            }

            fn captured_output(&self) -> String {
                "terminated fixture".into()
            }
        }

        let cache = absent_cache();
        fs::write(&cache, b"cache").unwrap();
        let mut supervisor = CoreSupervisor::default();
        supervisor
            .start(RoutingMode::Rules, false, &cache, &mut good_backend())
            .unwrap();
        assert_eq!(
            supervisor.poll(&mut Monitor(Some(9))).unwrap(),
            CorePoll::Exited
        );
        assert_eq!(supervisor.snapshot().desired_mode, RoutingMode::Rules);
        assert_eq!(supervisor.snapshot().applied_mode, None);
        assert!(supervisor.snapshot().traffic_may_be_direct);

        supervisor.retry(&cache, &mut good_backend()).unwrap();
        assert_eq!(supervisor.snapshot().status, CoreStatus::Running);
        assert_eq!(supervisor.snapshot().applied_mode, Some(RoutingMode::Rules));

        supervisor.mark_direct_recovered();
        assert_eq!(supervisor.snapshot().status, CoreStatus::Direct);
        assert_eq!(
            supervisor.snapshot().applied_mode,
            Some(RoutingMode::Direct)
        );
        assert!(!supervisor.snapshot().traffic_may_be_direct);
        let _ = fs::remove_file(cache);
    }

    #[test]
    fn existing_initialized_cache_allows_start() {
        let cache = absent_cache();
        fs::write(&cache, b"cache").unwrap();
        let mut supervisor = CoreSupervisor::default();
        supervisor
            .start(RoutingMode::Rules, true, &cache, &mut good_backend())
            .unwrap();
        let _ = fs::remove_file(cache);
    }

    #[test]
    fn readiness_without_a_created_cache_is_rejected() {
        let cache = absent_cache();
        let mut supervisor = CoreSupervisor::default();
        let error = supervisor
            .start(RoutingMode::Rules, false, &cache, &mut good_backend())
            .unwrap_err();
        assert_eq!(error.kind, CoreFailureKind::Readiness);
        assert!(!supervisor.snapshot().cache_initialized);
    }
}
