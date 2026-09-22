use super::{
    ApplicationRuntime, ApplyPlan, CompiledCoreConfig, CoreConfigInput, CoreConfigValidator,
    CoreSupervisor, DirectDnsServer, ProcessCoreBackend, ProxyCredentials,
};
use crate::{
    domain::{ProxyHost, ProxyProfile, RoutingRule},
    logs::{ConnectionEvent, ConnectionLogAdapter},
    routing::RoutingMode,
    storage::AppConfig,
};
use std::{
    fmt, io,
    net::{IpAddr, ToSocketAddrs},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[cfg(windows)]
const DNS_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub trait CoreCredentialSource: Send {
    fn load(&self, profile: &ProxyProfile) -> Result<Option<ProxyCredentials>, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoCredentials;

impl CoreCredentialSource for NoCredentials {
    fn load(&self, profile: &ProxyProfile) -> Result<Option<ProxyCredentials>, String> {
        if profile.auth_enabled {
            Err("当前代理需要凭据读取器".into())
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug)]
pub struct ManagedCoreRuntimeError(String);

impl fmt::Display for ManagedCoreRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ManagedCoreRuntimeError {}

impl From<io::Error> for ManagedCoreRuntimeError {
    fn from(error: io::Error) -> Self {
        Self(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeKey {
    mode: RoutingMode,
    profile: Option<ProxyProfile>,
    rules: Vec<RoutingRule>,
}

impl RuntimeKey {
    fn from_config(config: &AppConfig, mode: RoutingMode) -> Self {
        Self {
            mode,
            profile: (mode != RoutingMode::Direct)
                .then(|| config.profiles.active().cloned())
                .flatten(),
            rules: if mode == RoutingMode::Rules {
                config.rules.clone()
            } else {
                Vec::new()
            },
        }
    }
}

pub struct ManagedCoreRuntime<C> {
    core_path: PathBuf,
    runtime_directory: PathBuf,
    cache_path: PathBuf,
    direct_dns: DirectDnsServer,
    credential_source: C,
    backend: Option<ProcessCoreBackend>,
    supervisor: CoreSupervisor,
    applied_key: Option<RuntimeKey>,
    connection_logs: Arc<Mutex<ConnectionLogAdapter>>,
}

impl<C: CoreCredentialSource> ManagedCoreRuntime<C> {
    pub fn new(
        core_path: impl Into<PathBuf>,
        runtime_directory: impl Into<PathBuf>,
        cache_path: impl Into<PathBuf>,
        direct_dns: DirectDnsServer,
        credential_source: C,
    ) -> Self {
        let runtime_directory = runtime_directory.into();
        let connection_logs = Arc::new(Mutex::new(ConnectionLogAdapter::with_store(
            runtime_directory.join("connection-events.jsonl"),
        )));
        Self {
            core_path: core_path.into(),
            runtime_directory,
            cache_path: cache_path.into(),
            direct_dns,
            credential_source,
            backend: None,
            supervisor: CoreSupervisor::default(),
            applied_key: None,
            connection_logs,
        }
    }

    fn compiled_config(
        &self,
        config: &AppConfig,
        mode: RoutingMode,
    ) -> Result<CompiledCoreConfig, ManagedCoreRuntimeError> {
        let profile = config
            .profiles
            .require_active()
            .map_err(|error| ManagedCoreRuntimeError(error.to_string()))?;
        let credentials = self
            .credential_source
            .load(profile)
            .map_err(ManagedCoreRuntimeError)?;
        let upstream_addresses = resolve_upstream(profile)?;
        CompiledCoreConfig::compile(CoreConfigInput {
            profile,
            credentials: credentials.as_ref(),
            mode,
            rules: &config.rules,
            direct_dns: self.direct_dns,
            cache_path: &self.cache_path,
            control_endpoints: &[],
            upstream_addresses: &upstream_addresses,
        })
        .map_err(|error| ManagedCoreRuntimeError(error.to_string()))
    }

    fn stop(&mut self, flush_dns: bool) -> Result<(), ManagedCoreRuntimeError> {
        let was_running = self.backend.is_some();
        if let Some(mut backend) = self.backend.take()
            && let Err(error) = backend.stop()
        {
            // Keep pending TUN cleanup and its process job available for the
            // next explicit shutdown attempt instead of dropping the retry state.
            self.backend = Some(backend);
            return Err(ManagedCoreRuntimeError(error.to_string()));
        }
        if was_running && flush_dns {
            flush_system_dns_cache()?;
        }
        self.supervisor.mark_direct_recovered();
        Ok(())
    }

    fn apply_config(
        &mut self,
        config: &AppConfig,
        mode: RoutingMode,
    ) -> Result<(), ManagedCoreRuntimeError> {
        let key = RuntimeKey::from_config(config, mode);
        if self.applied_key.as_ref() == Some(&key) {
            return Ok(());
        }
        self.stop(mode == RoutingMode::Direct)?;
        if mode == RoutingMode::Direct {
            self.applied_key = Some(key);
            return Ok(());
        }
        let compiled = self.compiled_config(config, mode)?;
        let route_rule_ids = compiled.route_rule_ids().to_vec();
        let file = compiled
            .write_restricted(&self.runtime_directory)
            .map_err(|error| ManagedCoreRuntimeError(error.to_string()))?;
        self.connection_logs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .begin_revision(config.revision, route_rule_ids);
        let mut backend = ProcessCoreBackend::new(&self.core_path, file);
        let connection_logs = Arc::clone(&self.connection_logs);
        backend.set_line_observer(Arc::new(move |line| {
            connection_logs
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .ingest(line);
        }));
        self.supervisor
            .start(
                mode,
                config.cache_initialized,
                &self.cache_path,
                &mut backend,
            )
            .map_err(|error| ManagedCoreRuntimeError(error.to_string()))?;
        self.backend = Some(backend);
        self.applied_key = Some(key);
        Ok(())
    }
}

impl<C> Drop for ManagedCoreRuntime<C> {
    fn drop(&mut self) {
        if let Some(mut backend) = self.backend.take() {
            let _ = backend.stop();
            let _ = flush_system_dns_cache();
        }
    }
}

impl<C: CoreCredentialSource> ApplicationRuntime for ManagedCoreRuntime<C> {
    type Error = ManagedCoreRuntimeError;

    fn plan(
        &self,
        current: &AppConfig,
        current_mode: RoutingMode,
        candidate: &AppConfig,
        candidate_mode: RoutingMode,
    ) -> ApplyPlan {
        if RuntimeKey::from_config(current, current_mode)
            == RuntimeKey::from_config(candidate, candidate_mode)
        {
            ApplyPlan::Hot
        } else {
            ApplyPlan::Restart
        }
    }

    fn validate(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error> {
        if mode == RoutingMode::Direct {
            return Ok(());
        }
        let file = self
            .compiled_config(config, mode)?
            .write_restricted(&self.runtime_directory)
            .map_err(|error| ManagedCoreRuntimeError(error.to_string()))?;
        CoreConfigValidator::new(&self.core_path)
            .validate(file.path())
            .map_err(|error| ManagedCoreRuntimeError(error.to_string()))
    }

    fn apply(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error> {
        self.apply_config(config, mode)
    }

    fn rollback(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error> {
        self.applied_key = None;
        self.apply_config(config, mode)
    }

    fn shutdown(&mut self) -> Result<(), Self::Error> {
        self.stop(true)?;
        self.applied_key = None;
        Ok(())
    }

    fn cache_initialized(&self) -> Option<bool> {
        Some(self.supervisor.snapshot().cache_initialized)
    }

    fn connection_events(&self) -> Vec<ConnectionEvent> {
        self.connection_logs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .events()
    }

    fn clear_connection_events(&mut self) -> Result<(), String> {
        self.connection_logs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear()
            .map_err(|error| error.to_string())
    }

    fn poll_failure(&mut self) -> Result<Option<String>, String> {
        let Some(backend) = self.backend.as_mut() else {
            return Ok(None);
        };
        let Some(exit_code) = backend.try_wait().map_err(|error| error.to_string())? else {
            return Ok(None);
        };
        let output = backend.captured_output();
        self.supervisor.observe_exit(Some(exit_code), &output);
        self.backend = None;
        self.applied_key = None;
        Ok(Some(
            self.supervisor
                .snapshot()
                .failure
                .as_ref()
                .map(|failure| failure.to_string())
                .unwrap_or_else(|| "内核意外退出，流量可能直连".into()),
        ))
    }
}

#[cfg(windows)]
fn flush_system_dns_cache() -> Result<(), ManagedCoreRuntimeError> {
    use std::os::windows::process::CommandExt;
    use std::sync::atomic::{AtomicBool, Ordering};

    static TEST_TIMEOUT_CONSUMED: AtomicBool = AtomicBool::new(false);
    if std::env::var_os("SOCKS_PROXY_TEST_FORCE_DNS_FLUSH_TIMEOUT_ONCE").is_some()
        && !TEST_TIMEOUT_CONSUMED.swap(true, Ordering::SeqCst)
    {
        return Err(ManagedCoreRuntimeError(
            "刷新 Windows DNS 缓存超时（测试注入）".into(),
        ));
    }
    use std::{
        process::Command,
        thread,
        time::{Duration, Instant},
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut child = Command::new("ipconfig.exe")
        .arg("/flushdns")
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(ManagedCoreRuntimeError::from)?;
    let deadline = Instant::now() + DNS_FLUSH_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(ManagedCoreRuntimeError::from)? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ManagedCoreRuntimeError("刷新 Windows DNS 缓存超时".into()));
        }
        thread::sleep(Duration::from_millis(50));
    };
    if status.success() {
        Ok(())
    } else {
        Err(ManagedCoreRuntimeError(format!(
            "刷新 Windows DNS 缓存失败: {}",
            status
        )))
    }
}

#[cfg(not(windows))]
fn flush_system_dns_cache() -> Result<(), ManagedCoreRuntimeError> {
    Ok(())
}

fn resolve_upstream(profile: &ProxyProfile) -> Result<Vec<IpAddr>, ManagedCoreRuntimeError> {
    match &profile.host {
        ProxyHost::Ip(address) => Ok(vec![*address]),
        ProxyHost::Domain(domain) => {
            let addresses = (domain.as_str(), profile.port)
                .to_socket_addrs()
                .map_err(|error| {
                    ManagedCoreRuntimeError(format!("无法解析代理服务器地址: {error}"))
                })?
                .map(|address| address.ip())
                .collect::<Vec<_>>();
            if addresses.is_empty() {
                Err(ManagedCoreRuntimeError("代理服务器地址没有可用 IP".into()))
            } else {
                Ok(addresses)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ProfileId, ProxyHost, ProxyProfiles, ProxyProtocol};

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

    #[test]
    fn runtime_plan_restarts_only_when_effective_network_config_changes() {
        let first = profile("A", 1080);
        let first_id = first.id.clone();
        let mut profiles = ProxyProfiles::default();
        profiles.create(first).unwrap();
        profiles.select(&first_id).unwrap();
        let current = AppConfig {
            profiles,
            ..AppConfig::default()
        };
        let runtime = ManagedCoreRuntime::new(
            "sing-box",
            std::env::temp_dir(),
            std::env::temp_dir().join("cache.db"),
            DirectDnsServer {
                address: "1.1.1.1".parse().unwrap(),
                port: 53,
            },
            NoCredentials,
        );
        let mut candidate = current.clone();
        candidate.profiles.create(profile("B", 1081)).unwrap();
        assert_eq!(
            runtime.plan(&current, RoutingMode::Rules, &candidate, RoutingMode::Rules),
            ApplyPlan::Hot
        );
        candidate.profiles.get(&first_id).unwrap();
        let mut changed = candidate.profiles.get(&first_id).unwrap().clone();
        changed.port = 1090;
        candidate.profiles.update(changed).unwrap();
        assert_eq!(
            runtime.plan(&current, RoutingMode::Rules, &candidate, RoutingMode::Rules),
            ApplyPlan::Restart
        );
    }
}
