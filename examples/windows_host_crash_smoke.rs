#[cfg(not(windows))]
fn main() {
    panic!("this validation example only runs on Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use socks_proxy::{
        core::{DirectDnsServer, ManagedCoreRuntime, NoCredentials},
        domain::{ProfileId, ProxyHost, ProxyProfile, ProxyProfiles, ProxyProtocol},
        routing::RoutingMode,
        storage::{AppConfig, ConfigStore},
        ui::{ManagedDesktopController, MemoryCredentialVault, SharedController},
    };
    use std::{env, fs, path::PathBuf, thread, time::Duration};

    let action = env::args().nth(1).ok_or("missing action")?;
    let core = PathBuf::from(env::args_os().nth(2).ok_or("missing core path")?);
    let work = PathBuf::from(env::args_os().nth(3).ok_or("missing work directory")?);
    let config_path = work.join("host-crash-config.json");
    let cache_path = work.join("host-crash-cache.db");
    let runtime_path = work.join("host-crash-runtime");
    let store = ConfigStore::new(&config_path);

    if action == "run" {
        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_file(config_path.with_extension("json.bak"));
        let _ = fs::remove_file(&cache_path);
        let mut profiles = ProxyProfiles::default();
        let profile = ProxyProfile {
            id: ProfileId::new(),
            name: "Crash fixture".into(),
            protocol: ProxyProtocol::Socks5,
            host: ProxyHost::parse("127.0.0.1")?,
            port: 18141,
            auth_enabled: false,
            credential_ref: None,
        };
        let id = profile.id.clone();
        profiles.create(profile)?;
        profiles.select(&id)?;
        store.save(&AppConfig {
            profiles,
            last_applied_mode: RoutingMode::Rules,
            ..AppConfig::default()
        })?;
    }

    let config = store.load()?;
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
        config,
        runtime,
        ConfigStore::new(&config_path),
        MemoryCredentialVault::default(),
    )?;
    let restored = controller.state();

    match action.as_str() {
        "run" => {
            if restored.runtime.applied_mode != Some(RoutingMode::Rules) {
                return Err(
                    format!("startup did not restore Rules: {:?}", restored.runtime).into(),
                );
            }
            fs::write(
                work.join("host-crash-ready.json"),
                format!(
                    "{{\"host_pid\":{},\"restored_rules\":true}}",
                    std::process::id()
                ),
            )?;
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }
        "recover-direct" => {
            let restored_rules = restored.runtime.applied_mode == Some(RoutingMode::Rules)
                && restored.config.last_applied_mode == RoutingMode::Rules;
            controller.switch_mode(RoutingMode::Direct, true)?;
            let direct = controller.state();
            let direct_recovered = direct.runtime.applied_mode == Some(RoutingMode::Direct)
                && direct.config.last_applied_mode == RoutingMode::Direct;
            if !restored_rules || !direct_recovered {
                return Err("restart recovery result was invalid".into());
            }
            println!(
                "{{\"next_launch_restored_rules\":true,\"explicit_direct\":true,\"last_mode_direct\":true}}"
            );
            Ok(())
        }
        _ => Err("unknown action".into()),
    }
}
