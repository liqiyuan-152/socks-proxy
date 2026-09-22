#[cfg(not(windows))]
fn main() {
    panic!("this validation example only runs on Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use socks_proxy::logs::{ConnectionOutbound, ConnectionResult, RuleAttribution};
    use socks_proxy::{
        core::{
            CoreBackend, DirectDnsServer, ManagedCoreRuntime, NoCredentials, ProcessCoreBackend,
            RestrictedConfigFile,
        },
        domain::{ProfileId, ProxyHost, ProxyProfile, ProxyProfiles, ProxyProtocol},
        routing::RoutingMode,
        storage::AppConfig,
        ui::{
            ManagedDesktopController, MemoryCredentialVault, RuleDraft, RuleTargetKind,
            SharedController, UiControlError,
        },
    };
    use std::{
        env, fs, io,
        io::{Read, Write},
        net::TcpListener,
        path::{Path, PathBuf},
        process::Command,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Duration,
    };

    struct HttpFixture {
        stop: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl HttpFixture {
        fn start(port: u16, body: &'static str) -> io::Result<Self> {
            let listener = TcpListener::bind(("127.0.0.1", port))?;
            listener.set_nonblocking(true)?;
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = Arc::clone(&stop);
            let handle = thread::spawn(move || {
                while !thread_stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let mut request = [0u8; 4096];
                            let _ = stream.read(&mut request);
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = stream.write_all(response.as_bytes());
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20));
                        }
                        Err(_) => break,
                    }
                }
            });
            Ok(Self {
                stop,
                thread: Some(handle),
            })
        }
    }

    impl Drop for HttpFixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(handle) = self.thread.take() {
                let _ = handle.join();
            }
        }
    }

    #[derive(Default)]
    struct Store;

    impl socks_proxy::core::ApplicationStore for Store {
        type Error = io::Error;

        fn save_applied(&mut self, _config: &AppConfig) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn start_upstream(
        core: &Path,
        work: &Path,
        port: u16,
        target: u16,
    ) -> Result<ProcessCoreBackend, Box<dyn std::error::Error>> {
        let bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "log": {"level": "info", "timestamp": false},
            "inbounds": [{"type": "mixed", "tag": "upstream", "listen": "127.0.0.1", "listen_port": port}],
            "outbounds": [{"type": "direct", "tag": "direct"}],
            "route": {"rules": [{
                "inbound": ["upstream"], "action": "route", "outbound": "direct",
                "override_address": "127.0.0.1", "override_port": target
            }], "final": "direct"}
        }))?;
        let file = RestrictedConfigFile::create(work, &bytes)?;
        let mut backend = ProcessCoreBackend::new(core, file);
        let handle = backend.start()?;
        backend.wait_ready(handle)?;
        Ok(backend)
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

    fn probe(expected: &str) -> Result<(), Box<dyn std::error::Error>> {
        let output = Command::new("curl.exe")
            .args([
                "--silent",
                "--show-error",
                "--fail",
                "--max-time",
                "8",
                "--noproxy",
                "",
                "http://198.51.100.10/",
            ])
            .output()?;
        if !output.status.success() || String::from_utf8_lossy(&output.stdout) != expected {
            return Err(format!("expected path {expected}, curl status {}", output.status).into());
        }
        Ok(())
    }

    let core = PathBuf::from(env::args_os().nth(1).ok_or("missing sing-box path")?);
    let work = PathBuf::from(env::args_os().nth(2).ok_or("missing work directory")?);
    let runtime_directory = work.join("ui-controller-runtime");
    let cache_path = work.join("ui-controller-cache.db");
    let _ = fs::remove_file(&cache_path);
    let _a_target = HttpFixture::start(18124, "A")?;
    let _b_target = HttpFixture::start(18125, "B")?;
    let mut upstream_a = start_upstream(&core, &work, 18121, 18124)?;
    let mut upstream_b = start_upstream(&core, &work, 18122, 18125)?;

    let a = profile("A", 18121);
    let b = profile("B", 18122);
    let (a_id, b_id) = (a.id.clone(), b.id.clone());
    let mut profiles = ProxyProfiles::default();
    profiles.create(a)?;
    profiles.create(b)?;
    profiles.select(&a_id)?;
    let config = AppConfig {
        profiles,
        ..AppConfig::default()
    };
    let runtime = ManagedCoreRuntime::new(
        &core,
        &runtime_directory,
        &cache_path,
        DirectDnsServer {
            address: "1.1.1.1".parse()?,
            port: 53,
        },
        NoCredentials,
    );
    let mut controller =
        ManagedDesktopController::new(config, runtime, Store, MemoryCredentialVault::default())?;
    let rule_id = controller.save_rule(
        RuleDraft {
            name: "validation target".into(),
            target_kind: RuleTargetKind::Ip,
            target: "198.51.100.10".into(),
            ports: "80".into(),
            note: "created through shared UI controller".into(),
            ..RuleDraft::default()
        },
        true,
    )?;

    let rules_switched_without_confirmation =
        controller.switch_mode(RoutingMode::Rules, false).is_ok();
    probe("A")?;
    thread::sleep(Duration::from_millis(100));
    let rules_applied = controller.state().runtime.applied_mode == Some(RoutingMode::Rules);
    let rule_log = controller.state().connection_events.iter().any(|event| {
        event.outbound == ConnectionOutbound::Proxy
            && event.rule == RuleAttribution::Known(rule_id.clone())
            && event.result == ConnectionResult::Success
    });
    let rule_change_confirmation = matches!(
        controller.set_rule_enabled(&rule_id, false, false),
        Err(UiControlError::ConfirmationRequired(_))
    );

    let proxy_confirmation = matches!(
        controller.select_proxy(&b_id, false),
        Err(UiControlError::ConfirmationRequired(_))
    );
    controller.select_proxy(&b_id, true)?;
    probe("B")?;
    let b_applied = controller.state().config.profiles.active_id() == Some(&b_id);

    let global_switched_without_confirmation = controller
        .switch_mode(RoutingMode::GlobalProxy, false)
        .is_ok();
    probe("B")?;
    thread::sleep(Duration::from_millis(100));
    let global_applied = controller.state().runtime.applied_mode == Some(RoutingMode::GlobalProxy);
    let unknown_log = controller.state().connection_events.iter().any(|event| {
        event.outbound == ConnectionOutbound::Proxy
            && event.rule == RuleAttribution::Unknown
            && event.result == ConnectionResult::Success
    });

    upstream_b.stop()?;
    let failed_probe = Command::new("curl.exe")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--max-time",
            "3",
            "--noproxy",
            "",
            "http://198.51.100.10/",
        ])
        .output()?;
    thread::sleep(Duration::from_millis(100));
    let failure_log = !failed_probe.status.success()
        && controller.state().connection_events.iter().any(|event| {
            event.outbound == ConnectionOutbound::Proxy
                && matches!(event.result, ConnectionResult::Failure(_))
        });
    let mode_before_clear = controller.state().runtime.applied_mode;
    controller.clear_logs()?;
    let cleared_state = controller.state();
    let clear_kept_proxy = mode_before_clear == Some(RoutingMode::GlobalProxy)
        && cleared_state.runtime.applied_mode == mode_before_clear
        && cleared_state.connection_events.is_empty()
        && !runtime_directory.join("connection-events.jsonl").exists();

    controller.switch_mode(RoutingMode::Direct, true)?;
    fs::write(&cache_path, b"corrupt-cache")?;
    let failure = controller.switch_mode(RoutingMode::Rules, true).is_err();
    let failed_state = controller.state();
    let failure_kept_direct = failed_state.runtime.applied_mode == Some(RoutingMode::Direct)
        && failed_state.config.last_applied_mode == RoutingMode::Direct;

    drop(controller);
    upstream_a.stop()?;
    let _ = fs::remove_file(cache_path);
    if !rules_switched_without_confirmation
        || !rules_applied
        || !rule_change_confirmation
        || !proxy_confirmation
        || !b_applied
        || !global_switched_without_confirmation
        || !global_applied
        || !rule_log
        || !unknown_log
        || !failure_log
        || !clear_kept_proxy
        || !failure
        || !failure_kept_direct
    {
        return Err("unexpected shared controller result".into());
    }
    println!(
        "{{\"rules_switched_without_confirmation\":true,\"rules_path_a\":true,\"rule_change_confirmation\":true,\"proxy_confirmation\":true,\"path_b\":true,\"global_switched_without_confirmation\":true,\"global_path_b\":true,\"rule_log\":true,\"unknown_log\":true,\"failure_log\":true,\"clear_kept_proxy\":true,\"failure_visible\":true,\"failure_kept_direct\":true}}"
    );
    Ok(())
}
