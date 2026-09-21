#[cfg(not(windows))]
fn main() {
    panic!("this validation example only runs on Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use socks_proxy::{
        core::{CoreBackend, CoreStatus, CoreSupervisor, ProcessCoreBackend, RestrictedConfigFile},
        routing::RoutingMode,
    };
    use std::{env, fs, path::PathBuf};

    let core_path = PathBuf::from(env::args_os().nth(1).ok_or("missing sing-box path")?);
    let work_dir = PathBuf::from(env::args_os().nth(2).ok_or("missing work directory")?);
    let cache_path = work_dir.join("process-core-smoke-cache.db");
    let _ = fs::remove_file(&cache_path);
    let config_bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "log": {"level": "info", "timestamp": false},
        "inbounds": [{
            "type": "mixed",
            "tag": "local-probe",
            "listen": "127.0.0.1",
            "listen_port": 18110
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
    let config = RestrictedConfigFile::create(&work_dir, &config_bytes)?;
    let config_path = config.path().to_owned();
    let mut backend = ProcessCoreBackend::new(&core_path, config);
    let version = backend.version()?;
    let mut supervisor = CoreSupervisor::default();
    supervisor.start(RoutingMode::Rules, false, &cache_path, &mut backend)?;
    if supervisor.snapshot().status != CoreStatus::Running || !cache_path.is_file() {
        return Err("supervisor reported an invalid ready state".into());
    }
    let process_id = backend.try_wait()?;
    if process_id.is_some() {
        return Err("core exited after readiness".into());
    }
    backend.stop()?;
    if config_path.exists() {
        return Err("restricted config remained after core stop".into());
    }
    let _ = fs::remove_file(cache_path);
    println!(
        "{{\"version\":\"{}\",\"ready\":true,\"hidden_process\":true,\"stopped\":true,\"temporary_removed\":true}}",
        version
    );
    Ok(())
}
