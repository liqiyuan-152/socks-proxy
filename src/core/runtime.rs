use super::{
    ApplicationRuntime, ApplyPlan, CompiledCoreConfig, CoreConfigInput, CoreConfigValidator,
    CoreControlApi, CoreSupervisor, DirectDnsServer, ProcessCoreBackend, ProxyCredentials,
    RestrictedConfigFile,
};
use crate::{
    domain::{ProxyHost, ProxyProfile, RoutingRule},
    logs::{ConnectionEvent, ConnectionLogAdapter},
    routing::RoutingMode,
    storage::AppConfig,
};
use std::{
    fmt,
    io::{self, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

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
    profile: Option<ProxyProfile>,
    rules: Vec<RoutingRule>,
}

struct PreparedCoreConfig {
    key: RuntimeKey,
    mode: RoutingMode,
    compiled: CompiledCoreConfig,
    file: RestrictedConfigFile,
}

impl RuntimeKey {
    fn from_config(config: &AppConfig, mode: RoutingMode) -> Self {
        Self {
            profile: (mode != RoutingMode::Direct)
                .then(|| config.profiles.active().cloned())
                .flatten(),
            // The compiled non-direct configuration contains Rule and Global
            // branches, so both modes must compare the same configuration.
            rules: config.rules.clone(),
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
    control_api: Option<CoreControlApi>,
    prepared: Option<PreparedCoreConfig>,
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
            control_api: None,
            prepared: None,
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
        let control_api = CoreControlApi::new(
            available_loopback_endpoint()?,
            Uuid::new_v4().simple().to_string(),
        );
        let control_endpoints = vec![control_api.endpoint];
        CompiledCoreConfig::compile(CoreConfigInput {
            profile,
            credentials: credentials.as_ref(),
            mode,
            rules: &config.rules,
            direct_dns: self.direct_dns,
            cache_path: &self.cache_path,
            control_endpoints: &control_endpoints,
            control_api: Some(&control_api),
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
        self.control_api = None;
        self.supervisor.mark_direct_recovered();
        Ok(())
    }

    fn apply_config(
        &mut self,
        config: &AppConfig,
        mode: RoutingMode,
    ) -> Result<(), ManagedCoreRuntimeError> {
        let desired_key = RuntimeKey::from_config(config, mode);
        if self.applied_key.as_ref() == Some(&desired_key) {
            self.prepared = None;
            return Ok(());
        }
        if self.can_hot_switch(&desired_key, mode) {
            let result = self.update_mode(mode);
            self.prepared = None;
            result?;
            self.applied_key = Some(desired_key);
            return Ok(());
        }
        let prepared = if mode != RoutingMode::Direct {
            self.prepared
                .take()
                .filter(|prepared| prepared.mode == mode && prepared.key == desired_key)
        } else {
            None
        };
        self.stop(mode == RoutingMode::Direct)?;
        if mode == RoutingMode::Direct {
            self.applied_key = Some(desired_key);
            return Ok(());
        }
        let PreparedCoreConfig { compiled, file, .. } = match prepared {
            Some(prepared) => prepared,
            None => self.prepare(config, mode)?,
        };
        let route_rule_ids = compiled.route_rule_ids().to_vec();
        let control_api = compiled.control_api().cloned();
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
        self.control_api = control_api;
        self.applied_key = Some(desired_key);
        Ok(())
    }

    fn prepare(
        &self,
        config: &AppConfig,
        mode: RoutingMode,
    ) -> Result<PreparedCoreConfig, ManagedCoreRuntimeError> {
        let compiled = self.compiled_config(config, mode)?;
        let file = compiled
            .write_restricted(&self.runtime_directory)
            .map_err(|error| ManagedCoreRuntimeError(error.to_string()))?;
        CoreConfigValidator::new(&self.core_path)
            .validate(file.path())
            .map_err(|error| ManagedCoreRuntimeError(error.to_string()))?;
        Ok(PreparedCoreConfig {
            key: RuntimeKey::from_config(config, mode),
            mode,
            compiled,
            file,
        })
    }

    fn can_hot_switch(&self, desired_key: &RuntimeKey, mode: RoutingMode) -> bool {
        self.backend.is_some()
            && self.control_api.is_some()
            && mode != RoutingMode::Direct
            && self.applied_key.as_ref() == Some(desired_key)
    }

    fn update_mode(&self, mode: RoutingMode) -> Result<(), ManagedCoreRuntimeError> {
        let api = self
            .control_api
            .as_ref()
            .ok_or_else(|| ManagedCoreRuntimeError("运行内核不支持模式热切换".into()))?;
        let value = clash_mode(mode)?;
        let body = format!(r#"{{"mode":"{value}"}}"#);
        let response = control_request(api, "PATCH", Some(&body))?;
        if response.status != 204 {
            return Err(ManagedCoreRuntimeError(format!(
                "内核模式热切换失败（HTTP {}）",
                response.status
            )));
        }
        let response = control_request(api, "GET", None)?;
        if response.status != 200 {
            return Err(ManagedCoreRuntimeError(format!(
                "内核模式状态确认失败（HTTP {}）",
                response.status
            )));
        }
        let mode = serde_json::from_slice::<serde_json::Value>(&response.body)
            .ok()
            .and_then(|value| {
                value
                    .get("mode")
                    .and_then(|mode| mode.as_str())
                    .map(str::to_owned)
            });
        if mode.as_deref() != Some(value) {
            return Err(ManagedCoreRuntimeError("内核模式确认结果不一致".into()));
        }
        Ok(())
    }
}

struct ControlResponse {
    status: u16,
    body: Vec<u8>,
}

fn available_loopback_endpoint() -> Result<SocketAddr, ManagedCoreRuntimeError> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.local_addr().map_err(ManagedCoreRuntimeError::from)
}

fn clash_mode(mode: RoutingMode) -> Result<&'static str, ManagedCoreRuntimeError> {
    match mode {
        RoutingMode::Rules => Ok("Rule"),
        RoutingMode::GlobalProxy => Ok("Global"),
        RoutingMode::Direct => Err(ManagedCoreRuntimeError(
            "全局直连必须停止内核并恢复网络".into(),
        )),
    }
}

fn control_request(
    api: &CoreControlApi,
    method: &str,
    body: Option<&str>,
) -> Result<ControlResponse, ManagedCoreRuntimeError> {
    let mut stream = TcpStream::connect_timeout(&api.endpoint, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let body = body.unwrap_or("");
    let request = format!(
        "{method} /configs HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        api.endpoint,
        api.secret(),
        body.len()
    );
    stream.write_all(request.as_bytes())?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    let header_end = bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .ok_or_else(|| ManagedCoreRuntimeError("内核控制接口响应无效".into()))?;
    let status = std::str::from_utf8(&bytes[..header_end])
        .ok()
        .and_then(|headers| headers.lines().next())
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ManagedCoreRuntimeError("内核控制接口响应状态无效".into()))?;
    Ok(ControlResponse {
        status,
        body: bytes[(header_end + 4)..].to_vec(),
    })
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
            self.prepared = None;
            return Ok(());
        }
        self.prepared = Some(self.prepare(config, mode)?);
        Ok(())
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

    fn connection_log_page(
        &self,
        cursor: Option<crate::logs::ConnectionLogCursor>,
        filter: &crate::logs::ConnectionLogFilter,
    ) -> Result<crate::logs::ConnectionLogPage, String> {
        self.connection_logs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .page(cursor, filter, crate::logs::LOG_PAGE_SIZE)
            .map_err(|error| error.to_string())
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
        thread,
        time::{Duration, Instant},
    };

    let mut child = super::process::hidden_command(std::path::Path::new("ipconfig.exe"))
        .arg("/flushdns")
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
    use std::thread;

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
        assert_eq!(
            runtime.plan(
                &current,
                RoutingMode::Rules,
                &current,
                RoutingMode::GlobalProxy,
            ),
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

    fn serve_control_api(
        responses: Vec<&'static str>,
    ) -> (CoreControlApi, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let endpoint = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            responses
                .into_iter()
                .map(|response| {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut bytes = [0; 4096];
                    let length = stream.read(&mut bytes).unwrap();
                    stream.write_all(response.as_bytes()).unwrap();
                    String::from_utf8_lossy(&bytes[..length]).into_owned()
                })
                .collect()
        });
        (CoreControlApi::new(endpoint, "test-control-secret"), handle)
    }

    #[test]
    fn hot_mode_switch_uses_authenticated_patch_and_confirms_the_applied_mode() {
        let (api, server) = serve_control_api(vec![
            "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 17\r\nConnection: close\r\n\r\n{\"mode\":\"Global\"}",
        ]);
        let mut runtime = ManagedCoreRuntime::new(
            "sing-box",
            std::env::temp_dir(),
            std::env::temp_dir().join("cache.db"),
            DirectDnsServer {
                address: "1.1.1.1".parse().unwrap(),
                port: 53,
            },
            NoCredentials,
        );
        runtime.control_api = Some(api);

        runtime.update_mode(RoutingMode::GlobalProxy).unwrap();
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("PATCH /configs HTTP/1.1"));
        assert!(requests[0].contains("Authorization: Bearer test-control-secret"));
        assert!(requests[0].ends_with("{\"mode\":\"Global\"}"));
        assert!(requests[1].starts_with("GET /configs HTTP/1.1"));
    }

    #[test]
    fn hot_mode_switch_rejects_api_failure_and_confirmation_mismatch() {
        let (api, server) = serve_control_api(vec![
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ]);
        let mut runtime = ManagedCoreRuntime::new(
            "sing-box",
            std::env::temp_dir(),
            std::env::temp_dir().join("cache.db"),
            DirectDnsServer {
                address: "1.1.1.1".parse().unwrap(),
                port: 53,
            },
            NoCredentials,
        );
        runtime.control_api = Some(api);
        assert!(runtime.update_mode(RoutingMode::Rules).is_err());
        server.join().unwrap();

        let (api, server) = serve_control_api(vec![
            "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"mode\":\"Rule\"}",
        ]);
        runtime.control_api = Some(api);
        let error = runtime.update_mode(RoutingMode::GlobalProxy).unwrap_err();
        assert!(error.to_string().contains("确认结果不一致"));
        server.join().unwrap();
    }
}
