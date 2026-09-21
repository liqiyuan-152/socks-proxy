#[cfg(not(windows))]
fn main() {
    panic!("this validation example only runs on Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use socks_proxy::{
        core::{DirectDnsServer, ManagedCoreRuntime, NoCredentials},
        domain::{ProfileId, ProxyHost, ProxyProfile, ProxyProfiles, ProxyProtocol},
        logs::{ConnectionOutbound, ConnectionResult, RuleAttribution},
        routing::RoutingMode,
        storage::AppConfig,
        ui::{
            ManagedDesktopController, MemoryCredentialVault, RuleDraft, RuleTargetKind,
            SharedController, UiControlError,
        },
    };
    use std::{
        env, fs, io,
        net::IpAddr,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    #[derive(Default)]
    struct Store;

    impl socks_proxy::core::ApplicationStore for Store {
        type Error = io::Error;

        fn save_applied(&mut self, _config: &AppConfig) -> Result<(), Self::Error> {
            Ok(())
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

    fn start_ssh(
        target: IpAddr,
        target_user: &str,
        target_port: u16,
        key: &Path,
        command: &str,
        stdout_path: &Path,
        stderr_path: &Path,
    ) -> io::Result<Child> {
        let stdout = fs::File::create(stdout_path)?;
        let stderr = fs::File::create(stderr_path)?;
        Command::new("ssh.exe")
            .args([
                "-i",
                key.to_str()
                    .ok_or_else(|| io::Error::other("invalid key path"))?,
                "-o",
                "BatchMode=yes",
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=NUL",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "ServerAliveInterval=1",
                "-o",
                "ServerAliveCountMax=3",
                "-p",
                &target_port.to_string(),
                &format!("{target_user}@{target}"),
                command,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
    }

    fn wait_for_text(
        child: &mut Child,
        path: &Path,
        expected: &str,
        timeout: Duration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let deadline = Instant::now() + timeout;
        loop {
            if fs::read_to_string(path)
                .unwrap_or_default()
                .contains(expected)
            {
                return Ok(());
            }
            if let Some(status) = child.try_wait()? {
                return Err(format!("ssh exited before {expected}: {status}").into());
            }
            if Instant::now() >= deadline {
                return Err(format!("timed out waiting for ssh output {expected}").into());
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    let core = PathBuf::from(env::args_os().nth(1).ok_or("missing core path")?);
    let work = PathBuf::from(env::args_os().nth(2).ok_or("missing work directory")?);
    let target: IpAddr = env::args().nth(3).ok_or("missing SSH target")?.parse()?;
    let key = PathBuf::from(env::args_os().nth(4).ok_or("missing SSH key")?);
    let target_user = env::args().nth(5).ok_or("missing SSH user")?;
    let target_port: u16 = env::args().nth(6).ok_or("missing SSH port")?.parse()?;
    let cache_path = work.join("ssh-long-session-cache.db");
    let runtime_path = work.join("ssh-long-session-runtime");
    let _ = fs::remove_file(&cache_path);

    let a = profile("SSH upstream A", 18141);
    let b = profile("SSH upstream B", 18142);
    let (a_id, b_id) = (a.id.clone(), b.id.clone());
    let mut profiles = ProxyProfiles::default();
    profiles.create(a)?;
    profiles.create(b)?;
    profiles.select(&a_id)?;
    let runtime = ManagedCoreRuntime::new(
        &core,
        &runtime_path,
        &cache_path,
        DirectDnsServer {
            address: "1.1.1.1".parse()?,
            port: 53,
        },
        NoCredentials,
    );
    let mut controller = ManagedDesktopController::new(
        AppConfig {
            profiles,
            ..AppConfig::default()
        },
        runtime,
        Store,
        MemoryCredentialVault::default(),
    )?;
    let rule_id = controller.save_rule(
        RuleDraft {
            name: "SSH long session fixture".into(),
            target_kind: RuleTargetKind::Ip,
            target: target.to_string(),
            ports: target_port.to_string(),
            note: "Windows acceptance fixture".into(),
            ..RuleDraft::default()
        },
        false,
    )?;
    let rules_confirmation = matches!(
        controller.switch_mode(RoutingMode::Rules, false),
        Err(UiControlError::ConfirmationRequired(_))
    );
    controller.switch_mode(RoutingMode::Rules, true)?;

    let long_out = work.join("ssh-long.out");
    let long_err = work.join("ssh-long.err");
    let mut long = start_ssh(
        target,
        &target_user,
        target_port,
        &key,
        "echo READY; sleep 30",
        &long_out,
        &long_err,
    )?;
    wait_for_text(&mut long, &long_out, "READY", Duration::from_secs(15))?;
    let long_active_before_switch = long.try_wait()?.is_none();

    let proxy_confirmation = matches!(
        controller.select_proxy(&b_id, false),
        Err(UiControlError::ConfirmationRequired(_))
    );
    controller.select_proxy(&b_id, true)?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let long_disconnected_on_restart = loop {
        if long.try_wait()?.is_some() {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(100));
    };
    if !long_disconnected_on_restart {
        long.kill()?;
        let _ = long.wait();
    }

    let after_out = work.join("ssh-after.out");
    let after_err = work.join("ssh-after.err");
    let mut after = start_ssh(
        target,
        &target_user,
        target_port,
        &key,
        "echo AFTER",
        &after_out,
        &after_err,
    )?;
    wait_for_text(&mut after, &after_out, "AFTER", Duration::from_secs(15))?;
    let after_status = after.wait()?;
    thread::sleep(Duration::from_millis(300));
    let state = controller.state();
    let proxy_rule_events = state
        .connection_events
        .iter()
        .filter(|event| {
            event.target == target.to_string()
                && event.port == target_port
                && event.outbound == ConnectionOutbound::Proxy
                && event.rule == RuleAttribution::Known(rule_id.clone())
                && event.result == ConnectionResult::Success
        })
        .count();
    let upstream_a_seen = fs::read_to_string(work.join("ssh-upstream-18141.log"))
        .unwrap_or_default()
        .contains(&format!("inbound connection to {target}:{target_port}"));
    let upstream_b_seen = fs::read_to_string(work.join("ssh-upstream-18142.log"))
        .unwrap_or_default()
        .contains(&format!("inbound connection to {target}:{target_port}"));
    controller.switch_mode(RoutingMode::Direct, true)?;
    let direct = controller.state().runtime.applied_mode == Some(RoutingMode::Direct);

    let result = serde_json::json!({
        "rules_confirmation": rules_confirmation,
        "proxy_confirmation": proxy_confirmation,
        "long_active_before_switch": long_active_before_switch,
        "long_disconnected_on_restart": long_disconnected_on_restart,
        "long_survived_restart": !long_disconnected_on_restart,
            "new_ssh_after_switch": after_status.success(),
            "proxy_rule_events": proxy_rule_events,
            "upstream_a_seen": upstream_a_seen,
            "upstream_b_seen": upstream_b_seen,
            "explicit_direct": direct
    });
    println!("{result}");
    if !rules_confirmation
        || !proxy_confirmation
        || !long_active_before_switch
        || !after_status.success()
        || !upstream_a_seen
        || !upstream_b_seen
        || !direct
    {
        return Err(format!("SSH long-session acceptance result was incomplete: {result}").into());
    }
    Ok(())
}
