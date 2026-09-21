#[cfg(not(windows))]
fn main() {
    panic!("this validation example only runs on Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use socks_proxy::{
        core::{ApplicationStore, DirectDnsServer, ManagedCoreRuntime, NoCredentials},
        domain::{ProfileId, ProxyHost, ProxyProfile, ProxyProfiles, ProxyProtocol},
        routing::RoutingMode,
        storage::AppConfig,
        ui::{ManagedDesktopController, MemoryCredentialVault, SharedController, UiControlError},
    };
    use std::{
        env, io,
        net::{IpAddr, ToSocketAddrs},
        path::PathBuf,
        thread,
        time::{Duration, Instant},
    };

    #[derive(Default)]
    struct Store;

    impl ApplicationStore for Store {
        type Error = io::Error;

        fn save_applied(&mut self, _config: &AppConfig) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn is_fake(address: IpAddr) -> bool {
        match address {
            IpAddr::V4(address) => {
                let octets = address.octets();
                octets[0] == 198 && matches!(octets[1], 18 | 19)
            }
            IpAddr::V6(address) => address.segments()[0] & 0xffc0 == 0xfc00,
        }
    }

    fn resolve_until(
        domain: &str,
        expected_fake: bool,
        timeout: Duration,
    ) -> Result<Vec<IpAddr>, Box<dyn std::error::Error>> {
        let deadline = Instant::now() + timeout;
        loop {
            let addresses = (domain, 80)
                .to_socket_addrs()
                .map(|values| values.map(|value| value.ip()).collect::<Vec<_>>())
                .unwrap_or_default();
            if !addresses.is_empty()
                && addresses.iter().copied().all(is_fake) == expected_fake
                && (expected_fake || addresses.iter().copied().all(|address| !is_fake(address)))
            {
                return Ok(addresses);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "DNS did not reach expected fake={expected_fake}; last addresses: {addresses:?}"
                )
                .into());
            }
            thread::sleep(Duration::from_millis(250));
        }
    }

    let core = PathBuf::from(env::args_os().nth(1).ok_or("missing core path")?);
    let work = PathBuf::from(env::args_os().nth(2).ok_or("missing work directory")?);
    let direct_dns: IpAddr = env::args().nth(3).ok_or("missing direct DNS")?.parse()?;
    let domain = env::args()
        .nth(4)
        .unwrap_or_else(|| "example.com".to_owned());
    let upstream_port: u16 = env::args()
        .nth(5)
        .unwrap_or_else(|| "18141".to_owned())
        .parse()?;

    let profile = ProxyProfile {
        id: ProfileId::new(),
        name: "DNS recovery fixture".into(),
        protocol: ProxyProtocol::Socks5,
        host: ProxyHost::parse("127.0.0.1")?,
        port: upstream_port,
        auth_enabled: false,
        credential_ref: None,
    };
    let profile_id = profile.id.clone();
    let mut profiles = ProxyProfiles::default();
    profiles.create(profile)?;
    profiles.select(&profile_id)?;
    let runtime = ManagedCoreRuntime::new(
        &core,
        work.join("shutdown-dns-runtime"),
        work.join("shutdown-dns-cache.db"),
        DirectDnsServer {
            address: direct_dns,
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

    let confirmation_required = matches!(
        controller.switch_mode(RoutingMode::Rules, false),
        Err(UiControlError::ConfirmationRequired(_))
    );
    controller.switch_mode(RoutingMode::Rules, true)?;
    let fake_addresses = resolve_until(&domain, true, Duration::from_secs(15))?;

    controller.shutdown()?;
    let direct_addresses = resolve_until(&domain, false, Duration::from_secs(15))?;
    let state = controller.state();
    let last_mode_preserved = state.config.last_applied_mode == RoutingMode::Rules;
    let shutdown_direct = state.runtime.applied_mode == Some(RoutingMode::Direct);

    let result = serde_json::json!({
        "confirmation_required": confirmation_required,
        "fake_addresses": fake_addresses,
        "direct_addresses": direct_addresses,
        "shutdown_direct": shutdown_direct,
        "last_mode_preserved": last_mode_preserved,
        "application_cache_notice": true
    });
    println!("{result}");
    if !confirmation_required || !shutdown_direct || !last_mode_preserved {
        return Err(format!("shutdown DNS recovery result was incomplete: {result}").into());
    }
    Ok(())
}
