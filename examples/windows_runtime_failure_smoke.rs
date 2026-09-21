#[cfg(not(windows))]
fn main() {
    panic!("this validation example only runs on Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use socks_proxy::{
        core::{CorePoll, CoreStatus, CoreSupervisor, ProcessCoreBackend, RestrictedConfigFile},
        routing::RoutingMode,
    };
    use std::{
        env, fs,
        path::{Path, PathBuf},
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    fn backend(
        core_path: &Path,
        work_dir: &Path,
        cache_path: &Path,
        port: u16,
    ) -> Result<ProcessCoreBackend, Box<dyn std::error::Error>> {
        let bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "log": {"level": "info", "timestamp": false},
            "inbounds": [{
                "type": "mixed",
                "tag": "runtime-failure-probe",
                "listen": "127.0.0.1",
                "listen_port": port
            }],
            "outbounds": [{"type": "direct", "tag": "direct"}],
            "route": {"final": "direct"},
            "experimental": {"cache_file": {
                "enabled": true,
                "path": cache_path,
                "store_fakeip": true,
                "strict_mode": true
            }}
        }))?;
        let config = RestrictedConfigFile::create(work_dir, &bytes)?;
        Ok(ProcessCoreBackend::new(core_path, config))
    }

    let core_path = PathBuf::from(env::args_os().nth(1).ok_or("missing sing-box path")?);
    let work_dir = PathBuf::from(env::args_os().nth(2).ok_or("missing work directory")?);
    let cache_path = work_dir.join("runtime-failure-smoke-cache.db");
    let _ = fs::remove_file(&cache_path);

    let mut first = backend(&core_path, &work_dir, &cache_path, 18111)?;
    let mut supervisor = CoreSupervisor::default();
    let handle = supervisor.start(RoutingMode::Rules, false, &cache_path, &mut first)?;
    let status = Command::new("taskkill")
        .args(["/PID", &handle.0.to_string(), "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        return Err("failed to terminate validation core".into());
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match supervisor.poll(&mut first)? {
            CorePoll::Exited => break,
            CorePoll::Running if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            CorePoll::Running => return Err("core exit was not observed".into()),
        }
    }
    let error_visible = supervisor.snapshot().status == CoreStatus::Error
        && supervisor.snapshot().applied_mode.is_none()
        && supervisor.snapshot().desired_mode == RoutingMode::Rules
        && supervisor.snapshot().traffic_may_be_direct;
    first.stop()?;

    let mut second = backend(&core_path, &work_dir, &cache_path, 18111)?;
    supervisor.retry(&cache_path, &mut second)?;
    let retry_rules = supervisor.snapshot().status == CoreStatus::Running
        && supervisor.snapshot().desired_mode == RoutingMode::Rules
        && supervisor.snapshot().applied_mode == Some(RoutingMode::Rules)
        && !supervisor.snapshot().traffic_may_be_direct;
    second.stop()?;
    supervisor.mark_direct_recovered();
    let direct_recovery = supervisor.snapshot().status == CoreStatus::Direct
        && supervisor.snapshot().applied_mode == Some(RoutingMode::Direct)
        && !supervisor.snapshot().traffic_may_be_direct;

    let _ = fs::remove_file(cache_path);
    if !error_visible || !retry_rules || !direct_recovery {
        return Err("unexpected runtime recovery state".into());
    }
    println!(
        "{{\"exit_detected\":true,\"error_visible\":true,\"traffic_may_be_direct\":true,\"last_mode_preserved\":true,\"retry_rules\":true,\"direct_recovery\":true}}"
    );
    Ok(())
}
