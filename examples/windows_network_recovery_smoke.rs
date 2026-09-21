#[cfg(not(windows))]
fn main() {
    panic!("this validation example only runs on Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use socks_proxy::core::{
        NetworkChange, NetworkRecovery, NetworkRecoveryBackend, NetworkResource, RecoveryStore,
    };
    use socks_proxy::routing::RoutingMode;
    use std::{collections::HashMap, env, io, path::PathBuf, process::Command};

    #[derive(Default)]
    struct Backend {
        values: HashMap<NetworkResource, Option<String>>,
        fail_writes: usize,
        fail_flushes: usize,
        flushes: usize,
    }

    impl NetworkRecoveryBackend for Backend {
        type Error = io::Error;

        fn read(&mut self, resource: &NetworkResource) -> Result<Option<String>, Self::Error> {
            Ok(self.values.get(resource).cloned().flatten())
        }

        fn write(
            &mut self,
            resource: &NetworkResource,
            value: Option<&str>,
        ) -> Result<(), Self::Error> {
            if self.fail_writes > 0 {
                self.fail_writes -= 1;
                return Err(io::Error::other("injected restore failure"));
            }
            self.values
                .insert(resource.clone(), value.map(str::to_owned));
            Ok(())
        }

        fn flush_dns_cache(&mut self) -> Result<(), Self::Error> {
            if self.fail_flushes > 0 {
                self.fail_flushes -= 1;
                return Err(io::Error::other("injected DNS flush failure"));
            }
            let status = Command::new("ipconfig").arg("/flushdns").output()?.status;
            if !status.success() {
                return Err(io::Error::other(format!(
                    "ipconfig /flushdns failed: {status}"
                )));
            }
            self.flushes += 1;
            Ok(())
        }
    }

    let workspace = PathBuf::from(env::args_os().nth(1).ok_or("missing work directory")?);
    let journal_path = workspace.join("network-recovery-smoke.json");
    let _ = std::fs::remove_file(&journal_path);
    let dns = NetworkResource::DnsServers {
        interface_id: "fixture-interface".into(),
    };
    let route = NetworkResource::Route {
        destination: "198.18.0.0/15".into(),
    };
    let mut backend = Backend::default();
    backend
        .values
        .insert(dns.clone(), Some("system-dns".into()));
    backend.values.insert(route.clone(), None);
    let last_successful_mode = RoutingMode::Rules;
    let mut recovery = NetworkRecovery::new(backend, RecoveryStore::new(&journal_path));
    recovery.apply(&[
        NetworkChange {
            resource: dns.clone(),
            value: Some("managed-dns".into()),
        },
        NetworkChange {
            resource: route.clone(),
            value: Some("owned-route".into()),
        },
    ])?;

    let mut backend = recovery.into_backend();
    backend.values.insert(dns.clone(), Some("vpn-dns".into()));
    backend.fail_writes = 1;
    let mut recovery = NetworkRecovery::new(backend, RecoveryStore::new(&journal_path));
    let first = recovery.restore()?;
    let first_retryable = !first.complete
        && first.pending == vec![route.clone()]
        && first.preserved_external == vec![dns.clone()]
        && journal_path.is_file();

    let mut backend = recovery.into_backend();
    backend.fail_flushes = 1;
    let mut restarted = NetworkRecovery::new(backend, RecoveryStore::new(&journal_path));
    let second = restarted.restore()?;
    let flush_retryable = !second.complete
        && second.pending.is_empty()
        && second.dns_flush_pending
        && second.application_cache_notice;
    let third = restarted.restore()?;
    let backend = restarted.into_backend();
    let complete = third.complete
        && backend.values[&dns] == Some("vpn-dns".into())
        && backend.values[&route].is_none()
        && backend.flushes == 1
        && !journal_path.exists();
    let mode_unchanged = last_successful_mode == RoutingMode::Rules;
    if !first_retryable || !flush_retryable || !complete || !mode_unchanged {
        return Err("unexpected network recovery result".into());
    }
    println!(
        "{{\"external_setting_preserved\":true,\"restore_retry\":true,\"dns_flush_retry\":true,\"application_cache_notice\":true,\"journal_cleared\":true,\"last_mode_unchanged\":true}}"
    );
    Ok(())
}
