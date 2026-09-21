#[cfg(not(windows))]
fn main() {
    panic!("this validation example only runs on Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use socks_proxy::{
        core::{
            CompiledCoreConfig, CoreConfigInput, CoreConfigValidator, DirectDnsServer,
            ProxyCredentials,
        },
        domain::{
            CredentialRef, DomainName, PortSet, ProfileId, ProxyHost, ProxyProfile, ProxyProtocol,
            RoutingRule, RuleTarget,
        },
        routing::RoutingMode,
    };
    use std::{env, fs, path::PathBuf, process::Command, str::FromStr};

    let core_path = PathBuf::from(env::args_os().nth(1).ok_or("missing sing-box path")?);
    let work_dir = PathBuf::from(env::args_os().nth(2).ok_or("missing work directory")?);
    let secret = ProxyCredentials::new("fixture-user", "fixture-password-4.1");
    let profile = ProxyProfile {
        id: ProfileId::new(),
        name: "validation proxy".into(),
        protocol: ProxyProtocol::Socks5,
        host: ProxyHost::Domain(DomainName::parse("proxy.example")?),
        port: 1080,
        auth_enabled: true,
        credential_ref: Some(CredentialRef::parse("credential-validation")?),
    };
    let rules = [RoutingRule {
        id: "ssh-rule".into(),
        name: "SSH fixture".into(),
        enabled: true,
        target: RuleTarget::DomainSuffix(DomainName::parse("fixture.invalid")?),
        ports: PortSet::from_str("22,443,8000-9000")?,
        note: String::new(),
    }];
    let cache_path = work_dir.join("config-smoke-cache.db");
    let compiled = CompiledCoreConfig::compile(CoreConfigInput {
        profile: &profile,
        credentials: Some(&secret),
        mode: RoutingMode::Rules,
        rules: &rules,
        direct_dns: DirectDnsServer {
            address: "1.1.1.1".parse()?,
            port: 53,
        },
        cache_path: &cache_path,
        control_endpoints: &["127.0.0.1:19090".parse()?],
        upstream_addresses: &["203.0.113.9".parse()?],
    })?;
    let config = compiled.write_restricted(&work_dir)?;

    let validator = CoreConfigValidator::new(&core_path);
    let command = validator.command(config.path());
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    if args.contains("fixture-password-4.1") || args.contains("fixture-user") {
        return Err("credential leaked into core arguments".into());
    }
    validator.validate(config.path())?;

    let acl_script = r#"
$ErrorActionPreference = 'Stop'
$acl = Get-Acl -LiteralPath $env:SOCKS_PROXY_CONFIG_PATH
$current = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$allowed = @($current, 'S-1-5-18')
$seen = @{}
foreach ($rule in $acl.Access) {
    $sid = $rule.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
    if ($allowed -notcontains $sid -or $rule.AccessControlType -ne 'Allow' -or -not $rule.FileSystemRights.HasFlag([Security.AccessControl.FileSystemRights]::FullControl)) { exit 21 }
    $seen[$sid] = $true
}
if (-not $acl.AreAccessRulesProtected -or -not $seen[$current] -or -not $seen['S-1-5-18'] -or $seen.Count -ne 2) { exit 22 }
"#;
    let acl_status = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", acl_script])
        .env("SOCKS_PROXY_CONFIG_PATH", config.path())
        .status()?;
    if !acl_status.success() {
        return Err(format!("restricted ACL validation failed: {acl_status}").into());
    }

    let config_path = config.path().to_owned();
    drop(config);
    let temporary_removed = !config_path.exists();
    if !temporary_removed {
        return Err("temporary config was not removed".into());
    }
    let disk = fs::read_dir(&work_dir)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("core-"))
        .count();
    if disk != 0 {
        return Err("unexpected core temporary config remains".into());
    }

    println!(
        "{{\"config_check\":true,\"acl_current_user_and_system\":true,\"secret_in_args\":false,\"temporary_removed\":true}}"
    );
    Ok(())
}
