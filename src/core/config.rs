use crate::{
    domain::{ProxyHost, ProxyProfile, ProxyProtocol, RoutingRule, RuleTarget},
    routing::RoutingMode,
};
use serde_json::{Map, Value, json};
use std::{
    fmt, fs, io,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};
use uuid::Uuid;

const DIRECT_OUTBOUND: &str = "direct";
const PROXY_OUTBOUND: &str = "proxy";
const DIRECT_DNS: &str = "direct-dns";

pub struct ProxyCredentials {
    username: String,
    password: String,
}

impl ProxyCredentials {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }
}

impl fmt::Debug for ProxyCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProxyCredentials")
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .finish()
    }
}

impl Drop for ProxyCredentials {
    fn drop(&mut self) {
        unsafe {
            self.username.as_bytes_mut().fill(0);
            self.password.as_bytes_mut().fill(0);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectDnsServer {
    pub address: IpAddr,
    pub port: u16,
}

pub struct CoreConfigInput<'a> {
    pub profile: &'a ProxyProfile,
    pub credentials: Option<&'a ProxyCredentials>,
    pub mode: RoutingMode,
    pub rules: &'a [RoutingRule],
    pub direct_dns: DirectDnsServer,
    pub cache_path: &'a Path,
    pub control_endpoints: &'a [SocketAddr],
    pub upstream_addresses: &'a [IpAddr],
}

#[derive(Debug)]
pub enum CoreConfigError {
    Validation(&'static str),
    Json(serde_json::Error),
    Io(io::Error),
    CheckFailed(ExitStatus),
}

impl fmt::Display for CoreConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(message) => formatter.write_str(message),
            Self::Json(error) => write!(formatter, "内核配置序列化失败: {error}"),
            Self::Io(error) => write!(formatter, "内核配置文件操作失败: {error}"),
            Self::CheckFailed(status) => write!(formatter, "锁定内核拒绝配置: {status}"),
        }
    }
}

impl std::error::Error for CoreConfigError {}

impl From<io::Error> for CoreConfigError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub struct CompiledCoreConfig {
    bytes: Vec<u8>,
    route_rule_ids: Vec<Option<String>>,
}

impl fmt::Debug for CompiledCoreConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompiledCoreConfig")
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

impl Drop for CompiledCoreConfig {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

impl CompiledCoreConfig {
    pub fn compile(input: CoreConfigInput<'_>) -> Result<Self, CoreConfigError> {
        input
            .profile
            .validate()
            .map_err(|error| CoreConfigError::Validation(error.0))?;
        if input.mode == RoutingMode::Direct {
            return Err(CoreConfigError::Validation(
                "全局直连不启动内核，无需生成内核配置",
            ));
        }
        if input.direct_dns.port == 0 {
            return Err(CoreConfigError::Validation("直连 DNS 端口必须非零"));
        }
        for rule in input.rules {
            rule.validate()
                .map_err(|error| CoreConfigError::Validation(error.0))?;
        }
        validate_credentials(input.profile, input.credentials)?;

        let cache_path = input
            .cache_path
            .to_str()
            .ok_or(CoreConfigError::Validation("缓存路径不是有效 Unicode"))?;
        let mut route_rules = base_exception_rules(&input);
        let mut route_rule_ids = vec![None; route_rules.len()];
        if input.mode == RoutingMode::Rules {
            route_rule_ids.extend(user_route_rule_ids(input.rules, true));
            route_rules.extend(user_route_rules(input.rules, true));
            if input.rules.iter().any(is_enabled_ip_rule) {
                route_rules.push(direct_resolve_rule());
                route_rule_ids.push(None);
                route_rule_ids.extend(user_route_rule_ids(input.rules, false));
                route_rules.extend(user_route_rules(input.rules, false));
            }
        } else {
            route_rules.push(direct_resolve_rule());
            route_rule_ids.push(None);
            route_rules.push(json!({
                "ip_all_private": true,
                "action": "route",
                "outbound": DIRECT_OUTBOUND
            }));
            route_rule_ids.push(None);
        }

        let document = json!({
            "log": {"level": "debug", "timestamp": true},
            "dns": {
                "servers": [
                    {
                        "type": "fakeip",
                        "tag": "fakeip",
                        "inet4_range": "198.18.0.0/15",
                        "inet6_range": "fc00::/18"
                    },
                    {
                        "type": "udp",
                        "tag": DIRECT_DNS,
                        "server": input.direct_dns.address.to_string(),
                        "server_port": input.direct_dns.port
                    }
                ],
                "rules": [{
                    "query_type": ["A", "AAAA"],
                    "action": "route",
                    "server": "fakeip"
                }],
                "final": DIRECT_DNS,
                "reverse_mapping": true
            },
            "inbounds": [{
                "type": "tun",
                "tag": "tun-in",
                "interface_name": "socks-proxy-tun-v1",
                "address": ["172.30.255.1/30", "fdfe:dcba:9876::1/126"],
                "auto_route": true,
                "strict_route": false,
                "dns_mode": "hijack",
                "stack": "mixed"
            }],
            "outbounds": [
                {"type": "direct", "tag": DIRECT_OUTBOUND},
                proxy_outbound(input.profile, input.credentials)
            ],
            "route": {
                "auto_detect_interface": true,
                "default_domain_resolver": {"server": DIRECT_DNS},
                "rules": route_rules,
                "final": if input.mode == RoutingMode::Rules {
                    DIRECT_OUTBOUND
                } else {
                    PROXY_OUTBOUND
                }
            },
            "experimental": {
                "cache_file": {
                    "enabled": true,
                    "path": cache_path,
                    "store_fakeip": true,
                    "strict_mode": true
                }
            }
        });
        let bytes = serde_json::to_vec_pretty(&document).map_err(CoreConfigError::Json)?;
        Ok(Self {
            bytes,
            route_rule_ids,
        })
    }

    pub fn write_restricted(
        &self,
        directory: &Path,
    ) -> Result<RestrictedConfigFile, CoreConfigError> {
        RestrictedConfigFile::create(directory, &self.bytes)
    }

    pub fn route_rule_ids(&self) -> &[Option<String>] {
        &self.route_rule_ids
    }

    #[cfg(test)]
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).expect("compiled config is JSON")
    }
}

pub struct RestrictedConfigFile {
    pub(super) path: PathBuf,
}

impl RestrictedConfigFile {
    pub fn create(directory: &Path, bytes: &[u8]) -> Result<Self, CoreConfigError> {
        fs::create_dir_all(directory)?;
        let path = directory.join(format!("core-{}.json", Uuid::new_v4()));
        write_restricted_file(&path, bytes)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Debug for RestrictedConfigFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RestrictedConfigFile")
            .field("path", &self.path)
            .finish()
    }
}

impl Drop for RestrictedConfigFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub struct CoreConfigValidator<'a> {
    core_path: &'a Path,
}

impl<'a> CoreConfigValidator<'a> {
    pub fn new(core_path: &'a Path) -> Self {
        Self { core_path }
    }

    pub fn command(&self, config_path: &Path) -> Command {
        let mut command = Command::new(self.core_path);
        command.arg("check").arg("-c").arg(config_path);
        command
    }

    pub fn validate(&self, config_path: &Path) -> Result<(), CoreConfigError> {
        let output = self.command(config_path).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(CoreConfigError::CheckFailed(output.status))
        }
    }
}

fn validate_credentials(
    profile: &ProxyProfile,
    credentials: Option<&ProxyCredentials>,
) -> Result<(), CoreConfigError> {
    match (profile.auth_enabled, credentials) {
        (true, None) => Err(CoreConfigError::Validation("已启用认证但凭据不可用")),
        (false, Some(_)) => Err(CoreConfigError::Validation("未启用认证时不得注入凭据")),
        (true, Some(credentials))
            if credentials.username.is_empty() || credentials.password.is_empty() =>
        {
            Err(CoreConfigError::Validation("代理用户名和密码不能为空"))
        }
        _ => Ok(()),
    }
}

fn proxy_outbound(profile: &ProxyProfile, credentials: Option<&ProxyCredentials>) -> Value {
    let mut outbound = Map::from_iter([
        (
            "type".into(),
            Value::String(
                match profile.protocol {
                    ProxyProtocol::Socks5 => "socks",
                    ProxyProtocol::Http => "http",
                }
                .into(),
            ),
        ),
        ("tag".into(), Value::String(PROXY_OUTBOUND.into())),
        (
            "server".into(),
            Value::String(match &profile.host {
                ProxyHost::Ip(ip) => ip.to_string(),
                ProxyHost::Domain(domain) => domain.as_str().to_owned(),
            }),
        ),
        ("server_port".into(), Value::from(profile.port)),
    ]);
    if profile.protocol == ProxyProtocol::Socks5 {
        outbound.insert("version".into(), Value::String("5".into()));
    }
    if matches!(profile.host, ProxyHost::Domain(_)) {
        outbound.insert("domain_resolver".into(), json!({"server": DIRECT_DNS}));
    }
    if let Some(credentials) = credentials {
        outbound.insert(
            "username".into(),
            Value::String(credentials.username.clone()),
        );
        outbound.insert(
            "password".into(),
            Value::String(credentials.password.clone()),
        );
    }
    Value::Object(outbound)
}

fn base_exception_rules(input: &CoreConfigInput<'_>) -> Vec<Value> {
    let mut rules = Vec::new();
    match &input.profile.host {
        ProxyHost::Ip(ip) => {
            rules.push(direct_exception_rule(
                "ip_cidr",
                json!([host_cidr(*ip)]),
                input.profile.port,
            ));
        }
        ProxyHost::Domain(domain) => {
            rules.push(direct_exception_rule(
                "domain",
                json!([domain.as_str()]),
                input.profile.port,
            ));
            if !input.upstream_addresses.is_empty() {
                rules.push(direct_exception_rule(
                    "ip_cidr",
                    json!(
                        input
                            .upstream_addresses
                            .iter()
                            .copied()
                            .map(host_cidr)
                            .collect::<Vec<_>>()
                    ),
                    input.profile.port,
                ));
            }
        }
    }

    for endpoint in input.control_endpoints {
        rules.push(json!({
            "ip_cidr": [host_cidr(endpoint.ip())],
            "port": [endpoint.port()],
            "action": "route",
            "outbound": DIRECT_OUTBOUND
        }));
    }
    rules.push(direct_exception_rule(
        "ip_cidr",
        json!([host_cidr(input.direct_dns.address)]),
        input.direct_dns.port,
    ));
    rules
}

fn direct_exception_rule(field: &str, target: Value, port: u16) -> Value {
    let mut rule = Map::from_iter([
        (field.into(), target),
        ("port".into(), json!([port])),
        ("action".into(), Value::String("route".into())),
        ("outbound".into(), Value::String(DIRECT_OUTBOUND.into())),
    ]);
    Value::Object(std::mem::take(&mut rule))
}

fn user_route_rules(rules: &[RoutingRule], domains: bool) -> Vec<Value> {
    rules
        .iter()
        .filter(|rule| {
            rule.enabled
                && (matches!(
                    rule.target,
                    RuleTarget::Domain(_) | RuleTarget::DomainSuffix(_)
                ) == domains)
        })
        .flat_map(|rule| {
            let mut value = Map::new();
            match &rule.target {
                RuleTarget::Domain(domain) => {
                    value.insert("domain".into(), json!([domain.as_str()]));
                }
                RuleTarget::DomainSuffix(domain) => {
                    value.insert("domain_suffix".into(), json!([domain.as_str()]));
                }
                RuleTarget::Ip(ip) => {
                    value.insert("ip_cidr".into(), json!([host_cidr(*ip)]));
                }
                RuleTarget::Cidr(network) => {
                    value.insert("ip_cidr".into(), json!([network.to_string()]));
                }
                RuleTarget::Range(range) => {
                    value.insert(
                        "ip_cidr".into(),
                        json!(
                            range
                                .to_cidrs()
                                .into_iter()
                                .map(|network| network.to_string())
                                .collect::<Vec<_>>()
                        ),
                    );
                }
            }
            value.insert("action".into(), Value::String("route".into()));
            value.insert("outbound".into(), Value::String(PROXY_OUTBOUND.into()));
            route_rule_port_variants(value, rule)
        })
        .collect()
}

fn user_route_rule_ids(rules: &[RoutingRule], domains: bool) -> Vec<Option<String>> {
    rules
        .iter()
        .filter(|rule| {
            rule.enabled
                && (matches!(
                    rule.target,
                    RuleTarget::Domain(_) | RuleTarget::DomainSuffix(_)
                ) == domains)
        })
        .flat_map(|rule| std::iter::repeat_n(Some(rule.id.clone()), route_rule_variant_count(rule)))
        .collect()
}

fn route_rule_variant_count(rule: &RoutingRule) -> usize {
    if rule.ports.intervals() == [(1, u16::MAX)] {
        1
    } else {
        usize::from(
            rule.ports
                .intervals()
                .iter()
                .any(|(start, end)| start == end),
        ) + usize::from(
            rule.ports
                .intervals()
                .iter()
                .any(|(start, end)| start != end),
        )
    }
}

fn is_enabled_ip_rule(rule: &RoutingRule) -> bool {
    rule.enabled
        && matches!(
            rule.target,
            RuleTarget::Ip(_) | RuleTarget::Cidr(_) | RuleTarget::Range(_)
        )
}

fn direct_resolve_rule() -> Value {
    json!({
        "network": ["tcp", "udp"],
        "action": "resolve",
        "server": DIRECT_DNS,
        "timeout": "5s"
    })
}

fn route_rule_port_variants(value: Map<String, Value>, rule: &RoutingRule) -> Vec<Value> {
    if rule.ports.intervals() == [(1, u16::MAX)] {
        return vec![Value::Object(value)];
    }
    let mut singles = Vec::new();
    let mut ranges = Vec::new();
    for &(start, end) in rule.ports.intervals() {
        if start == end {
            singles.push(start);
        } else {
            ranges.push(format!("{start}:{end}"));
        }
    }
    let mut variants = Vec::new();
    if !singles.is_empty() {
        let mut single_value = value.clone();
        single_value.insert("port".into(), json!(singles));
        variants.push(Value::Object(single_value));
    }
    if !ranges.is_empty() {
        let mut range_value = value;
        range_value.insert("port_range".into(), json!(ranges));
        variants.push(Value::Object(range_value));
    }
    variants
}

fn host_cidr(ip: IpAddr) -> String {
    format!("{ip}/{}", if ip.is_ipv4() { 32 } else { 128 })
}

#[cfg(not(windows))]
fn write_restricted_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(windows)]
fn write_restricted_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::{
        ffi::c_void,
        io::Write,
        mem,
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, ERROR_INSUFFICIENT_BUFFER, GENERIC_WRITE, GetLastError, LocalFree,
        },
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SDDL_REVISION_1,
            },
            PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::{CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_TEMPORARY, FILE_SHARE_READ},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    struct Handle(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle(token);
    let mut required = 0;
    unsafe {
        windows_sys::Win32::Security::GetTokenInformation(
            token.0,
            TokenUser,
            ptr::null_mut(),
            0,
            &mut required,
        )
    };
    if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return Err(io::Error::last_os_error());
    }
    let mut token_info = vec![0u8; required as usize];
    if unsafe {
        windows_sys::Win32::Security::GetTokenInformation(
            token.0,
            TokenUser,
            token_info.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let token_user = unsafe { &*(token_info.as_ptr().cast::<TOKEN_USER>()) };
    let mut sid_string = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_string) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let sid_len = unsafe { (0..).find(|&index| *sid_string.add(index) == 0).unwrap() };
    let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid_string, sid_len) });
    unsafe { LocalFree(sid_string.cast()) };

    let sddl: Vec<u16> = format!("D:P(A;;FA;;;SY)(A;;FA;;;{sid})")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.cast::<c_void>(),
        bInheritHandle: 0,
    };
    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ,
            &mut attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_TEMPORARY,
            ptr::null_mut(),
        )
    };
    unsafe { LocalFree(descriptor.cast()) };
    if handle.is_null() || handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { fs::File::from_raw_handle(handle.cast()) };
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CredentialRef, DomainName, IpNetwork, IpRange, PortSet, ProfileId};
    use std::{net::Ipv4Addr, str::FromStr};

    fn profile(authenticated: bool) -> ProxyProfile {
        ProxyProfile {
            id: ProfileId::new(),
            name: "fixture".into(),
            protocol: ProxyProtocol::Socks5,
            host: ProxyHost::Domain(DomainName::parse("proxy.example").unwrap()),
            port: 1080,
            auth_enabled: authenticated,
            credential_ref: authenticated
                .then(|| CredentialRef::parse("credential-fixture").unwrap()),
        }
    }

    fn rule(id: &str, target: RuleTarget, ports: &str) -> RoutingRule {
        RoutingRule {
            id: id.into(),
            name: id.into(),
            enabled: true,
            target,
            ports: PortSet::from_str(ports).unwrap(),
            note: String::new(),
        }
    }

    fn compile<'a>(
        profile: &'a ProxyProfile,
        credentials: Option<&'a ProxyCredentials>,
        rules: &'a [RoutingRule],
        mode: RoutingMode,
        cache: &'a Path,
    ) -> CompiledCoreConfig {
        CompiledCoreConfig::compile(CoreConfigInput {
            profile,
            credentials,
            mode,
            rules,
            direct_dns: DirectDnsServer {
                address: "192.0.2.53".parse().unwrap(),
                port: 53,
            },
            cache_path: cache,
            control_endpoints: &["127.0.0.1:19090".parse().unwrap()],
            upstream_addresses: &["203.0.113.9".parse().unwrap()],
        })
        .unwrap()
    }

    #[test]
    fn compiles_all_targets_ports_base_exceptions_and_strict_cache() {
        let profile = profile(false);
        let rules = vec![
            rule(
                "domain",
                RuleTarget::Domain(DomainName::parse("exact.example").unwrap()),
                "22,443,8000-9000",
            ),
            rule(
                "suffix",
                RuleTarget::DomainSuffix(DomainName::parse("suffix.example").unwrap()),
                "",
            ),
            rule("ip", RuleTarget::Ip("203.0.113.10".parse().unwrap()), "22"),
            rule(
                "cidr",
                RuleTarget::Cidr(IpNetwork::from_str("2001:db8::/32").unwrap()),
                "443",
            ),
            rule(
                "range",
                RuleTarget::Range(
                    IpRange::new(
                        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
                        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 30)),
                    )
                    .unwrap(),
                ),
                "",
            ),
        ];
        let compiled = compile(
            &profile,
            None,
            &rules,
            RoutingMode::Rules,
            Path::new("C:/cache/fakeip.db"),
        );
        let json = compiled.json();
        let route_rules = json["route"]["rules"].as_array().unwrap();
        assert_eq!(route_rules[0]["domain"], json!(["proxy.example"]));
        assert_eq!(route_rules[0]["outbound"], DIRECT_OUTBOUND);
        assert_eq!(route_rules[1]["ip_cidr"], json!(["203.0.113.9/32"]));
        assert_eq!(route_rules[2]["port"], json!([19090]));
        assert_eq!(route_rules[3]["ip_cidr"], json!(["192.0.2.53/32"]));
        assert_eq!(route_rules[3]["port"], json!([53]));
        assert_eq!(route_rules[4]["domain"], json!(["exact.example"]));
        assert_eq!(route_rules[4]["port"], json!([22, 443]));
        assert!(route_rules[4].get("port_range").is_none());
        assert_eq!(route_rules[5]["domain"], json!(["exact.example"]));
        assert_eq!(route_rules[5]["port_range"], json!(["8000:9000"]));
        assert!(route_rules[5].get("port").is_none());
        assert_eq!(route_rules[6]["domain_suffix"], json!(["suffix.example"]));
        assert!(route_rules[6].get("port").is_none());
        assert_eq!(route_rules[7]["action"], "resolve");
        assert_eq!(route_rules[7]["server"], DIRECT_DNS);
        assert_eq!(route_rules[8]["ip_cidr"], json!(["203.0.113.10/32"]));
        assert_eq!(route_rules[9]["ip_cidr"], json!(["2001:db8::/32"]));
        assert!(route_rules[10]["ip_cidr"].as_array().unwrap().len() > 1);
        assert_eq!(json["route"]["final"], DIRECT_OUTBOUND);
        assert_eq!(json["experimental"]["cache_file"]["strict_mode"], true);
        assert_eq!(compiled.route_rule_ids().len(), route_rules.len());
        assert_eq!(compiled.route_rule_ids()[4].as_deref(), Some("domain"));
        assert_eq!(compiled.route_rule_ids()[5].as_deref(), Some("domain"));
        assert_eq!(compiled.route_rule_ids()[6].as_deref(), Some("suffix"));
        assert_eq!(compiled.route_rule_ids()[7], None);
        assert_eq!(compiled.route_rule_ids()[8].as_deref(), Some("ip"));
        assert_eq!(compiled.route_rule_ids()[9].as_deref(), Some("cidr"));
        assert_eq!(compiled.route_rule_ids()[10].as_deref(), Some("range"));
    }

    #[test]
    fn credentials_are_required_only_for_authenticated_profiles_and_redacted() {
        let authenticated = profile(true);
        let secret = ProxyCredentials::new("fixture-user", "fixture-password");
        assert!(format!("{secret:?}").contains("[REDACTED]"));
        assert!(!format!("{secret:?}").contains("fixture-password"));
        assert!(
            CompiledCoreConfig::compile(CoreConfigInput {
                profile: &authenticated,
                credentials: None,
                mode: RoutingMode::Rules,
                rules: &[],
                direct_dns: DirectDnsServer {
                    address: "1.1.1.1".parse().unwrap(),
                    port: 53
                },
                cache_path: Path::new("cache.db"),
                control_endpoints: &[],
                upstream_addresses: &[],
            })
            .is_err()
        );
        let compiled = compile(
            &authenticated,
            Some(&secret),
            &[],
            RoutingMode::Rules,
            Path::new("cache.db"),
        );
        assert!(!format!("{compiled:?}").contains("fixture-password"));
        assert_eq!(
            compiled.json()["outbounds"][1]["password"],
            "fixture-password"
        );
    }

    #[test]
    fn global_mode_places_private_exception_before_proxy_final() {
        let profile = profile(false);
        let compiled = compile(
            &profile,
            None,
            &[],
            RoutingMode::GlobalProxy,
            Path::new("cache.db"),
        );
        let json = compiled.json();
        assert_eq!(json["route"]["rules"][4]["action"], "resolve");
        assert_eq!(json["route"]["rules"][5]["ip_all_private"], true);
        assert_eq!(json["route"]["final"], PROXY_OUTBOUND);
    }

    #[test]
    fn domain_only_rules_do_not_force_direct_resolution() {
        let profile = profile(false);
        let rules = [rule(
            "domain",
            RuleTarget::Domain(DomainName::parse("proxy-only.example").unwrap()),
            "443",
        )];
        let compiled = compile(
            &profile,
            None,
            &rules,
            RoutingMode::Rules,
            Path::new("cache.db"),
        );
        let route_rules = compiled.json()["route"]["rules"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(route_rules[4]["domain"], json!(["proxy-only.example"]));
        assert!(route_rules.iter().all(|rule| rule["action"] != "resolve"));
    }

    #[cfg(not(windows))]
    #[test]
    fn restricted_file_is_private_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;
        let profile = profile(false);
        let compiled = compile(
            &profile,
            None,
            &[],
            RoutingMode::Rules,
            Path::new("cache.db"),
        );
        let directory = std::env::temp_dir().join(format!("core-config-{}", Uuid::new_v4()));
        let file = compiled.write_restricted(&directory).unwrap();
        let path = file.path().to_owned();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        drop(file);
        assert!(!path.exists());
        fs::remove_dir(directory).unwrap();
    }
}
