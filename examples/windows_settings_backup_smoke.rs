#[cfg(not(windows))]
fn main() {
    panic!("this validation example only runs on Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use socks_proxy::{
        core::{ApplicationRuntime, ApplicationStore, ApplyPlan},
        routing::RoutingMode,
        storage::AppConfig,
        ui::{
            ManagedDesktopController, MemoryCredentialVault, ProxyDraft, RuleDraft,
            SharedController,
        },
    };
    use std::io;

    #[derive(Default)]
    struct Runtime;

    impl ApplicationRuntime for Runtime {
        type Error = io::Error;

        fn plan(
            &self,
            _current: &AppConfig,
            _current_mode: RoutingMode,
            _candidate: &AppConfig,
            _candidate_mode: RoutingMode,
        ) -> ApplyPlan {
            ApplyPlan::Hot
        }

        fn validate(&mut self, _config: &AppConfig, _mode: RoutingMode) -> Result<(), Self::Error> {
            Ok(())
        }

        fn apply(&mut self, _config: &AppConfig, _mode: RoutingMode) -> Result<(), Self::Error> {
            Ok(())
        }

        fn rollback(&mut self, _config: &AppConfig, _mode: RoutingMode) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct Store;

    impl ApplicationStore for Store {
        type Error = io::Error;

        fn save_applied(&mut self, _config: &AppConfig) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn controller() -> Result<
        ManagedDesktopController<Runtime, Store, MemoryCredentialVault>,
        Box<dyn std::error::Error>,
    > {
        Ok(ManagedDesktopController::new(
            AppConfig::default(),
            Runtime,
            Store,
            MemoryCredentialVault::default(),
        )?)
    }

    let mut source = controller()?;
    let initial = source.state();
    let first_launch_direct = initial.runtime.applied_mode == Some(RoutingMode::Direct)
        && initial.config.last_applied_mode == RoutingMode::Direct;
    let startup_disabled = !initial.config.preferences.start_with_windows;

    let mut proxy = ProxyDraft::default();
    proxy.name = "Authenticated fixture".into();
    proxy.host = "127.0.0.1".into();
    proxy.port = "1080".into();
    proxy.auth_enabled = true;
    proxy.username = "backup-fixture-user".into();
    proxy.password = "backup-fixture-password".into();
    source.save_proxy(proxy, false)?;
    source.save_rule(
        RuleDraft {
            name: "HTTPS fixture".into(),
            target: "example.com".into(),
            ports: "443".into(),
            ..RuleDraft::default()
        },
        false,
    )?;

    let backup = source.export_backup()?;
    let json = String::from_utf8(backup.clone())?;
    let secret_free = !json.contains("backup-fixture-user")
        && !json.contains("backup-fixture-password")
        && !json.contains("credential_ref")
        && !json.contains("last_applied_mode");
    let preview = source.preview_backup(&backup)?;
    let preview_complete =
        preview.profiles == 1 && preview.rules == 1 && preview.missing_credentials == 1;

    let mut target = controller()?;
    target.import_backup(&backup)?;
    let imported = target.state();
    let import_applied_direct = imported.config.profiles.iter().count() == 1
        && imported.config.rules.len() == 1
        && imported.runtime.applied_mode == Some(RoutingMode::Direct)
        && imported.config.last_applied_mode == RoutingMode::Direct;

    if !first_launch_direct
        || !startup_disabled
        || !secret_free
        || !preview_complete
        || !import_applied_direct
    {
        return Err("unexpected settings backup result".into());
    }
    println!(
        "{{\"first_launch_direct\":true,\"startup_disabled\":true,\"secret_free\":true,\"preview_missing_credentials\":true,\"import_applied_direct\":true}}"
    );
    Ok(())
}
