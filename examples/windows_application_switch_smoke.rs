#[cfg(not(windows))]
fn main() {
    panic!("this validation example only runs on Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use socks_proxy::{
        core::{
            ApplicationController, ApplicationRuntime, ApplicationStore, ApplyPlan, CoreBackend,
            CoreConfigValidator, ProcessCoreBackend, RestrictedConfigFile, SwitchError,
            SwitchRequest,
        },
        domain::{ProfileId, ProxyHost, ProxyProfile, ProxyProfiles, ProxyProtocol},
        routing::RoutingMode,
        storage::AppConfig,
    };
    use std::{
        env, fs, io,
        io::{Read, Write},
        net::{TcpListener, TcpStream},
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

    struct Runtime {
        core: PathBuf,
        work: PathBuf,
        client: Option<ProcessCoreBackend>,
        fail_next_apply: Arc<AtomicBool>,
    }

    impl Runtime {
        fn config(config: &AppConfig, mode: RoutingMode) -> Result<Vec<u8>, io::Error> {
            if mode == RoutingMode::Direct {
                return Ok(Vec::new());
            }
            let profile = config.profiles.require_active().map_err(io::Error::other)?;
            serde_json::to_vec_pretty(&serde_json::json!({
                "log": {"level": "info", "timestamp": false},
                "inbounds": [{"type": "mixed", "tag": "client", "listen": "127.0.0.1", "listen_port": 18103}],
                "outbounds": [{
                    "type": "socks",
                    "tag": "proxy",
                    "server": match profile.host { ProxyHost::Ip(ip) => ip.to_string(), _ => unreachable!() },
                    "server_port": profile.port,
                    "version": "5"
                }],
                "route": {"final": "proxy"}
            }))
            .map_err(io::Error::other)
        }

        fn stop(&mut self) -> Result<(), io::Error> {
            if let Some(mut client) = self.client.take() {
                client.stop().map_err(io::Error::other)?;
            }
            Ok(())
        }

        fn start(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), io::Error> {
            self.stop()?;
            if mode == RoutingMode::Direct {
                return Ok(());
            }
            if self.fail_next_apply.swap(false, Ordering::SeqCst) {
                return Err(io::Error::other("injected client restart failure"));
            }
            let bytes = Self::config(config, mode)?;
            let file =
                RestrictedConfigFile::create(&self.work, &bytes).map_err(io::Error::other)?;
            let mut backend = ProcessCoreBackend::new(&self.core, file);
            backend.version().map_err(io::Error::other)?;
            let handle = backend.start().map_err(io::Error::other)?;
            backend.wait_ready(handle).map_err(io::Error::other)?;
            self.client = Some(backend);
            Ok(())
        }
    }

    impl Drop for Runtime {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    impl ApplicationRuntime for Runtime {
        type Error = io::Error;

        fn plan(&self, _: &AppConfig, _: RoutingMode, _: &AppConfig, _: RoutingMode) -> ApplyPlan {
            ApplyPlan::Restart
        }

        fn validate(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error> {
            if mode == RoutingMode::Direct {
                return Ok(());
            }
            let bytes = Self::config(config, mode)?;
            let file =
                RestrictedConfigFile::create(&self.work, &bytes).map_err(io::Error::other)?;
            CoreConfigValidator::new(&self.core)
                .validate(file.path())
                .map_err(io::Error::other)
        }

        fn apply(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error> {
            self.start(config, mode)
        }

        fn rollback(&mut self, config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error> {
            self.start(config, mode)
        }
    }

    #[derive(Default)]
    struct Store(Vec<AppConfig>);
    impl ApplicationStore for Store {
        type Error = io::Error;
        fn save_applied(&mut self, config: &AppConfig) -> Result<(), Self::Error> {
            self.0.push(config.clone());
            Ok(())
        }
    }

    fn start_core(
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
                "5",
                "--noproxy",
                "no-bypass.invalid",
                "--proxy",
                "http://127.0.0.1:18103",
                "http://switch.fixture.invalid/",
            ])
            .output()?;
        if !output.status.success() || String::from_utf8_lossy(&output.stdout) != expected {
            return Err(format!("expected path {expected}, curl status {}", output.status).into());
        }
        Ok(())
    }

    let core = PathBuf::from(env::args_os().nth(1).ok_or("missing sing-box path")?);
    let work = PathBuf::from(env::args_os().nth(2).ok_or("missing work directory")?);
    let _a_target = HttpFixture::start(18104, "A")?;
    let _b_target = HttpFixture::start(18105, "B")?;
    let mut upstream_a = start_core(&core, &work, 18101, 18104)?;
    let mut upstream_b = start_core(&core, &work, 18102, 18105)?;

    let a = profile("A", 18101);
    let b = profile("B", 18102);
    let (a_id, b_id) = (a.id.clone(), b.id.clone());
    let mut profiles = ProxyProfiles::default();
    profiles.create(a)?;
    profiles.create(b)?;
    profiles.select(&a_id)?;
    let initial = AppConfig {
        profiles,
        ..AppConfig::default()
    };
    let fail_next = Arc::new(AtomicBool::new(false));
    let runtime = Runtime {
        core,
        work: work.clone(),
        client: None,
        fail_next_apply: Arc::clone(&fail_next),
    };
    let mut controller = ApplicationController::new(initial.clone(), runtime, Store::default())?;

    let confirmation = controller
        .switch(SwitchRequest {
            config: initial.clone(),
            mode: RoutingMode::Rules,
            restart_confirmed: false,
        })
        .unwrap_err();
    if !matches!(confirmation, SwitchError::RestartConfirmationRequired(_)) {
        return Err("restart warning was not required".into());
    }
    controller.switch(SwitchRequest {
        config: initial.clone(),
        mode: RoutingMode::Rules,
        restart_confirmed: true,
    })?;
    probe("A")?;

    let mut selected_b = controller.current_config().clone();
    selected_b.profiles.select(&b_id)?;
    controller.switch(SwitchRequest {
        config: selected_b.clone(),
        mode: RoutingMode::GlobalProxy,
        restart_confirmed: true,
    })?;
    probe("B")?;

    let mut selected_a = controller.current_config().clone();
    selected_a.profiles.select(&a_id)?;
    fail_next.store(true, Ordering::SeqCst);
    let failure = controller
        .switch(SwitchRequest {
            config: selected_a,
            mode: RoutingMode::Rules,
            restart_confirmed: true,
        })
        .unwrap_err();
    if !matches!(failure, SwitchError::Runtime(_))
        || controller.snapshot().applied_mode != Some(RoutingMode::GlobalProxy)
    {
        return Err("failed switch did not preserve the prior applied mode".into());
    }
    probe("B")?;

    controller.switch(SwitchRequest {
        config: controller.current_config().clone(),
        mode: RoutingMode::Direct,
        restart_confirmed: true,
    })?;
    thread::sleep(Duration::from_millis(200));
    if TcpStream::connect(("127.0.0.1", 18103)).is_ok() {
        return Err("client listener remained after Direct switch".into());
    }
    drop(controller);
    upstream_a.stop()?;
    upstream_b.stop()?;
    for entry in fs::read_dir(&work)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("core-") {
            return Err("restricted configuration remained after switching".into());
        }
    }
    println!(
        "{{\"path_a\":true,\"path_b\":true,\"rollback_b\":true,\"restart_warning\":true,\"direct_stopped\":true}}"
    );
    Ok(())
}
