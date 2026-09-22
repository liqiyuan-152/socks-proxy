use crate::{
    core::{
        ApplicationController, ApplicationRuntime, ApplicationSnapshot, ApplicationStore,
        SwitchError, SwitchRequest,
    },
    domain::{
        CredentialRef, DeleteContext, DeleteProfileError, DomainName, IpNetwork, IpRange, PortSet,
        ProfileId, ProxyHost, ProxyProfile, ProxyProtocol, RoutingRule, RuleTarget,
    },
    routing::RoutingMode,
    storage::{AppConfig, CredentialVault, export_backup, preview_import},
};
use std::fmt;
use std::{collections::BTreeSet, io, net::IpAddr, str::FromStr};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiState {
    pub config: AppConfig,
    pub runtime: ApplicationSnapshot,
    pub connection_events: Vec<crate::logs::ConnectionEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupPreview {
    pub profiles: usize,
    pub rules: usize,
    pub missing_credentials: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyDraft {
    pub id: Option<ProfileId>,
    pub name: String,
    pub protocol: ProxyProtocol,
    pub host: String,
    pub port: String,
    pub auth_enabled: bool,
    pub username: String,
    pub password: String,
}

impl Default for ProxyDraft {
    fn default() -> Self {
        Self {
            id: None,
            name: String::new(),
            protocol: ProxyProtocol::Socks5,
            host: String::new(),
            port: "1080".into(),
            auth_enabled: false,
            username: String::new(),
            password: String::new(),
        }
    }
}

impl ProxyDraft {
    pub fn from_profile(profile: &ProxyProfile) -> Self {
        Self {
            id: Some(profile.id.clone()),
            name: profile.name.clone(),
            protocol: profile.protocol,
            host: match &profile.host {
                ProxyHost::Ip(ip) => ip.to_string(),
                ProxyHost::Domain(domain) => domain.as_str().to_owned(),
            },
            port: profile.port.to_string(),
            auth_enabled: profile.auth_enabled,
            username: String::new(),
            password: String::new(),
        }
    }
}

impl Drop for ProxyDraft {
    fn drop(&mut self) {
        unsafe {
            self.password.as_bytes_mut().fill(0);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleTargetKind {
    Domain,
    DomainSuffix,
    Ip,
    Cidr,
    Range,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDraft {
    pub id: Option<String>,
    pub name: String,
    pub enabled: bool,
    pub target_kind: RuleTargetKind,
    pub target: String,
    pub ports: String,
    pub note: String,
}

impl Default for RuleDraft {
    fn default() -> Self {
        Self {
            id: None,
            name: String::new(),
            enabled: true,
            target_kind: RuleTargetKind::Domain,
            target: String::new(),
            ports: String::new(),
            note: String::new(),
        }
    }
}

impl RuleDraft {
    pub fn from_rule(rule: &RoutingRule) -> Self {
        let (target_kind, target) = match &rule.target {
            RuleTarget::Domain(value) => (RuleTargetKind::Domain, value.as_str().to_owned()),
            RuleTarget::DomainSuffix(value) => {
                (RuleTargetKind::DomainSuffix, value.as_str().to_owned())
            }
            RuleTarget::Ip(value) => (RuleTargetKind::Ip, value.to_string()),
            RuleTarget::Cidr(value) => (RuleTargetKind::Cidr, value.to_string()),
            RuleTarget::Range(value) => (RuleTargetKind::Range, value.to_string()),
        };
        Self {
            id: Some(rule.id.clone()),
            name: rule.name.clone(),
            enabled: rule.enabled,
            target_kind,
            target,
            ports: display_ports(&rule.ports),
            note: rule.note.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiControlError {
    Validation {
        field: &'static str,
        message: String,
    },
    ConfirmationRequired(String),
    Operation(String),
}

impl fmt::Display for UiControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation { message, .. }
            | Self::ConfirmationRequired(message)
            | Self::Operation(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for UiControlError {}

pub trait SharedController: Send {
    fn state(&mut self) -> UiState;
    fn shutdown(&mut self) -> Result<(), UiControlError>;
    fn switch_mode(
        &mut self,
        mode: RoutingMode,
        restart_confirmed: bool,
    ) -> Result<(), UiControlError>;
    fn select_proxy(
        &mut self,
        id: &ProfileId,
        restart_confirmed: bool,
    ) -> Result<(), UiControlError>;
    fn save_proxy(
        &mut self,
        draft: ProxyDraft,
        restart_confirmed: bool,
    ) -> Result<ProfileId, UiControlError>;
    fn delete_proxy(&mut self, id: &ProfileId) -> Result<(), UiControlError>;
    fn save_rule(
        &mut self,
        draft: RuleDraft,
        restart_confirmed: bool,
    ) -> Result<String, UiControlError>;
    fn delete_rule(&mut self, id: &str, restart_confirmed: bool) -> Result<(), UiControlError>;
    fn set_rule_enabled(
        &mut self,
        id: &str,
        enabled: bool,
        restart_confirmed: bool,
    ) -> Result<(), UiControlError>;
    fn export_backup(&self) -> Result<Vec<u8>, UiControlError>;
    fn preview_backup(&self, bytes: &[u8]) -> Result<BackupPreview, UiControlError>;
    fn import_backup(&mut self, bytes: &[u8]) -> Result<(), UiControlError>;
    fn clear_logs(&mut self) -> Result<(), UiControlError>;
}

#[derive(Default)]
pub struct DirectOnlyRuntime;

impl ApplicationRuntime for DirectOnlyRuntime {
    type Error = io::Error;

    fn plan(
        &self,
        _current: &AppConfig,
        current_mode: RoutingMode,
        _candidate: &AppConfig,
        candidate_mode: RoutingMode,
    ) -> crate::core::ApplyPlan {
        if current_mode == RoutingMode::Direct && candidate_mode == RoutingMode::Direct {
            crate::core::ApplyPlan::Hot
        } else {
            crate::core::ApplyPlan::Restart
        }
    }

    fn validate(&mut self, _config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error> {
        if mode == RoutingMode::Direct {
            Ok(())
        } else {
            Err(io::Error::other("网络运行时尚未初始化，未应用代理模式"))
        }
    }

    fn apply(&mut self, _config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error> {
        self.validate(_config, mode)
    }

    fn rollback(&mut self, _config: &AppConfig, mode: RoutingMode) -> Result<(), Self::Error> {
        self.validate(_config, mode)
    }
}

#[derive(Default)]
pub struct MemoryCredentialVault {
    references: BTreeSet<String>,
}

impl CredentialVault for MemoryCredentialVault {
    type Error = io::Error;

    fn create_version(&mut self, _secret: &[u8]) -> Result<CredentialRef, Self::Error> {
        let value = format!("credential-{}", Uuid::new_v4());
        self.references.insert(value.clone());
        CredentialRef::parse(&value).map_err(|error| io::Error::other(error.to_string()))
    }

    fn delete_version(&mut self, reference: &CredentialRef) -> Result<(), Self::Error> {
        self.references.remove(reference.as_str());
        Ok(())
    }
}

pub struct ManagedDesktopController<R, S, V> {
    application: ApplicationController<R, S>,
    vault: V,
}

impl<R, S, V> ManagedDesktopController<R, S, V>
where
    R: ApplicationRuntime,
    S: ApplicationStore,
    V: CredentialVault,
{
    pub fn new(config: AppConfig, runtime: R, store: S, vault: V) -> Result<Self, UiControlError> {
        let application = ApplicationController::new_restoring(config, runtime, store)
            .map_err(|error| UiControlError::Operation(error.to_string()))?;
        Ok(Self { application, vault })
    }

    fn apply(
        &mut self,
        config: AppConfig,
        mode: RoutingMode,
        restart_confirmed: bool,
    ) -> Result<(), UiControlError> {
        if mode != RoutingMode::Direct && config.profiles.active().is_none() {
            return self
                .application
                .stage_without_runtime(config, mode)
                .map_err(map_switch_error);
        }
        self.application
            .switch(SwitchRequest {
                config,
                mode,
                restart_confirmed,
            })
            .map(|_| ())
            .map_err(map_switch_error)
    }

    fn profile_from_draft(
        &mut self,
        draft: &ProxyDraft,
        previous: Option<&ProxyProfile>,
    ) -> Result<(ProxyProfile, Option<CredentialRef>), UiControlError> {
        if draft.name.trim().is_empty() {
            return Err(validation("name", "代理名称不能为空"));
        }
        let host =
            ProxyHost::parse(&draft.host).map_err(|error| validation("host", error.to_string()))?;
        let port = draft
            .port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| validation("port", "端口必须位于 1–65535"))?;
        let mut created_reference = None;
        let credential_ref = if draft.auth_enabled {
            if draft.username.is_empty() || draft.password.is_empty() {
                previous
                    .filter(|profile| profile.auth_enabled)
                    .and_then(|profile| profile.credential_ref.clone())
                    .ok_or_else(|| validation("credentials", "用户名和密码不能为空"))?
            } else {
                let mut secret = serde_json::to_vec(&(&draft.username, &draft.password))
                    .map_err(|error| UiControlError::Operation(error.to_string()))?;
                let result = self
                    .vault
                    .create_version(&secret)
                    .map_err(|error| UiControlError::Operation(error.to_string()));
                secret.fill(0);
                let reference = result?;
                created_reference = Some(reference.clone());
                reference
            }
        } else {
            if !draft.username.is_empty() || !draft.password.is_empty() {
                return Err(validation("credentials", "启用认证后才能保存用户名和密码"));
            }
            CredentialRef::parse("unused").expect("static non-secret reference is valid")
        };
        Ok((
            ProxyProfile {
                id: draft.id.clone().unwrap_or_default(),
                name: draft.name.trim().to_owned(),
                protocol: draft.protocol,
                host,
                port,
                auth_enabled: draft.auth_enabled,
                credential_ref: draft.auth_enabled.then_some(credential_ref),
            },
            created_reference,
        ))
    }

    fn rule_from_draft(draft: &RuleDraft) -> Result<RoutingRule, UiControlError> {
        if draft.name.trim().is_empty() {
            return Err(validation("name", "规则名称不能为空"));
        }
        let target = match draft.target_kind {
            RuleTargetKind::Domain => DomainName::parse(&draft.target).map(RuleTarget::Domain),
            RuleTargetKind::DomainSuffix => {
                DomainName::parse(&draft.target).map(RuleTarget::DomainSuffix)
            }
            RuleTargetKind::Ip => draft
                .target
                .trim()
                .parse::<IpAddr>()
                .map(RuleTarget::Ip)
                .map_err(|_| crate::domain::ValidationError("IP 地址无效")),
            RuleTargetKind::Cidr => IpNetwork::from_str(&draft.target).map(RuleTarget::Cidr),
            RuleTargetKind::Range => parse_ip_range(&draft.target).map(RuleTarget::Range),
        }
        .map_err(|error| validation("target", error.to_string()))?;
        let ports = PortSet::from_str(&draft.ports)
            .map_err(|error| validation("ports", error.to_string()))?;
        let rule = RoutingRule {
            id: draft
                .id
                .clone()
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            name: draft.name.trim().to_owned(),
            enabled: draft.enabled,
            target,
            ports,
            note: draft.note.trim().to_owned(),
        };
        rule.validate()
            .map_err(|error| validation("rule", error.to_string()))?;
        Ok(rule)
    }
}

impl<R, S, V> SharedController for ManagedDesktopController<R, S, V>
where
    R: ApplicationRuntime + Send,
    S: ApplicationStore + Send,
    V: CredentialVault + Send,
{
    fn state(&mut self) -> UiState {
        self.application.refresh_runtime();
        UiState {
            config: self.application.current_config().clone(),
            runtime: self.application.snapshot().clone(),
            connection_events: self.application.connection_events(),
        }
    }

    fn shutdown(&mut self) -> Result<(), UiControlError> {
        self.application.shutdown().map_err(map_switch_error)
    }

    fn switch_mode(
        &mut self,
        mode: RoutingMode,
        _restart_confirmed: bool,
    ) -> Result<(), UiControlError> {
        // An explicit mode selection is sufficient authorization for a restart.
        self.apply(self.application.current_config().clone(), mode, true)
    }

    fn select_proxy(
        &mut self,
        id: &ProfileId,
        restart_confirmed: bool,
    ) -> Result<(), UiControlError> {
        let mut candidate = self.application.current_config().clone();
        candidate
            .profiles
            .select(id)
            .map_err(|error| validation("active_proxy", error.to_string()))?;
        let mode = self.application.snapshot().desired_mode;
        self.apply(candidate, mode, restart_confirmed)
    }

    fn save_proxy(
        &mut self,
        draft: ProxyDraft,
        restart_confirmed: bool,
    ) -> Result<ProfileId, UiControlError> {
        let mut candidate = self.application.current_config().clone();
        let previous = draft
            .id
            .as_ref()
            .and_then(|id| candidate.profiles.get(id))
            .cloned();
        let (profile, created_reference) = self.profile_from_draft(&draft, previous.as_ref())?;
        let id = profile.id.clone();
        let creating_first_profile = previous.is_none() && candidate.profiles.active_id().is_none();
        let result = if previous.is_some() {
            candidate.profiles.update(profile)
        } else {
            candidate.profiles.create(profile)
        };
        if let Err(error) = result {
            if let Some(reference) = &created_reference {
                let _ = self.vault.delete_version(reference);
            }
            return Err(validation("profile", error.to_string()));
        }
        if creating_first_profile {
            candidate
                .profiles
                .select(&id)
                .map_err(|error| validation("active_proxy", error.to_string()))?;
        }
        let mode = self.application.snapshot().desired_mode;
        if let Err(error) = self.apply(candidate, mode, restart_confirmed || creating_first_profile)
        {
            if let Some(reference) = &created_reference {
                let _ = self.vault.delete_version(reference);
            }
            return Err(error);
        }
        Ok(id)
    }

    fn delete_proxy(&mut self, id: &ProfileId) -> Result<(), UiControlError> {
        let mut candidate = self.application.current_config().clone();
        let context = if self.application.snapshot().applied_mode == Some(RoutingMode::Direct) {
            DeleteContext::Direct
        } else {
            DeleteContext::Proxying
        };
        candidate
            .profiles
            .delete(id, context)
            .map_err(|error| match error {
                DeleteProfileError::ActiveProfile => UiControlError::Operation(
                    "当前代理正在承载流量，请先选择其他代理或切换全局直连".into(),
                ),
                DeleteProfileError::NotFound => UiControlError::Operation("代理不存在".into()),
            })?;
        let mode = self.application.snapshot().desired_mode;
        self.apply(candidate, mode, true)
    }

    fn save_rule(
        &mut self,
        draft: RuleDraft,
        restart_confirmed: bool,
    ) -> Result<String, UiControlError> {
        let mut candidate = self.application.current_config().clone();
        let rule = Self::rule_from_draft(&draft)?;
        let id = rule.id.clone();
        if let Some(index) = candidate.rules.iter().position(|item| item.id == id) {
            candidate.rules[index] = rule;
        } else {
            candidate.rules.push(rule);
        }
        let mode = self.application.snapshot().desired_mode;
        self.apply(candidate, mode, restart_confirmed)?;
        Ok(id)
    }

    fn delete_rule(&mut self, id: &str, restart_confirmed: bool) -> Result<(), UiControlError> {
        let mut candidate = self.application.current_config().clone();
        let before = candidate.rules.len();
        candidate.rules.retain(|rule| rule.id != id);
        if candidate.rules.len() == before {
            return Err(UiControlError::Operation("规则不存在".into()));
        }
        let mode = self.application.snapshot().desired_mode;
        self.apply(candidate, mode, restart_confirmed)
    }

    fn set_rule_enabled(
        &mut self,
        id: &str,
        enabled: bool,
        restart_confirmed: bool,
    ) -> Result<(), UiControlError> {
        let mut candidate = self.application.current_config().clone();
        let rule = candidate
            .rules
            .iter_mut()
            .find(|rule| rule.id == id)
            .ok_or_else(|| UiControlError::Operation("规则不存在".into()))?;
        rule.enabled = enabled;
        let mode = self.application.snapshot().desired_mode;
        self.apply(candidate, mode, restart_confirmed)
    }

    fn export_backup(&self) -> Result<Vec<u8>, UiControlError> {
        export_backup(self.application.current_config())
            .map_err(|error| UiControlError::Operation(error.to_string()))
    }

    fn preview_backup(&self, bytes: &[u8]) -> Result<BackupPreview, UiControlError> {
        let preview = preview_import(bytes, self.application.current_config().revision)
            .map_err(|error| UiControlError::Operation(error.to_string()))?;
        let replacement = preview.replacement();
        Ok(BackupPreview {
            profiles: replacement.profiles.iter().count(),
            rules: replacement.rules.len(),
            missing_credentials: replacement
                .profiles
                .iter()
                .filter(|profile| profile.auth_enabled && profile.credential_ref.is_none())
                .count(),
        })
    }

    fn import_backup(&mut self, bytes: &[u8]) -> Result<(), UiControlError> {
        let snapshot = self.application.snapshot();
        if snapshot.phase != crate::core::SwitchPhase::Direct
            || snapshot.applied_mode != Some(RoutingMode::Direct)
        {
            return Err(UiControlError::Operation(
                "仅可在网络已恢复的全局直连状态导入".into(),
            ));
        }
        let preview = preview_import(bytes, self.application.current_config().revision)
            .map_err(|error| UiControlError::Operation(error.to_string()))?;
        self.apply(preview.replacement().clone(), RoutingMode::Direct, true)
    }

    fn clear_logs(&mut self) -> Result<(), UiControlError> {
        self.application
            .clear_connection_events()
            .map_err(UiControlError::Operation)
    }
}

fn parse_ip_range(value: &str) -> Result<IpRange, crate::domain::ValidationError> {
    let (start, end) = value
        .trim()
        .split_once('-')
        .or_else(|| value.trim().split_once('–'))
        .ok_or(crate::domain::ValidationError("IP 范围需要起点和终点"))?;
    let start = start
        .trim()
        .parse()
        .map_err(|_| crate::domain::ValidationError("IP 范围起点无效"))?;
    let end = end
        .trim()
        .parse()
        .map_err(|_| crate::domain::ValidationError("IP 范围终点无效"))?;
    IpRange::new(start, end)
}

fn display_ports(ports: &PortSet) -> String {
    if ports.intervals() == [(1, u16::MAX)] {
        return String::new();
    }
    ports
        .intervals()
        .iter()
        .map(|(start, end)| {
            if start == end {
                start.to_string()
            } else {
                format!("{start}-{end}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn validation(field: &'static str, message: impl Into<String>) -> UiControlError {
    UiControlError::Validation {
        field,
        message: message.into(),
    }
}

fn map_switch_error(error: SwitchError) -> UiControlError {
    match error {
        SwitchError::RestartConfirmationRequired(message) => {
            UiControlError::ConfirmationRequired(message.into())
        }
        SwitchError::Invalid(message) => validation("mode", message),
        other => UiControlError::Operation(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ApplyPlan;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    struct Runtime {
        fail_apply: Arc<AtomicBool>,
    }

    impl ApplicationRuntime for Runtime {
        type Error = io::Error;

        fn plan(
            &self,
            current: &AppConfig,
            current_mode: RoutingMode,
            candidate: &AppConfig,
            candidate_mode: RoutingMode,
        ) -> ApplyPlan {
            let active_profile_unchanged = current.profiles.active() == candidate.profiles.active();
            let rules_unchanged = current.rules == candidate.rules;
            if current_mode == candidate_mode && active_profile_unchanged && rules_unchanged {
                ApplyPlan::Hot
            } else {
                ApplyPlan::Restart
            }
        }

        fn validate(&mut self, _config: &AppConfig, _mode: RoutingMode) -> Result<(), Self::Error> {
            Ok(())
        }

        fn apply(&mut self, _config: &AppConfig, _mode: RoutingMode) -> Result<(), Self::Error> {
            if self.fail_apply.load(Ordering::SeqCst) {
                Err(io::Error::other("injected apply failure"))
            } else {
                Ok(())
            }
        }

        fn rollback(&mut self, _config: &AppConfig, _mode: RoutingMode) -> Result<(), Self::Error> {
            Ok(())
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

    fn draft(name: &str, port: u16) -> ProxyDraft {
        let mut draft = ProxyDraft::default();
        draft.name = name.into();
        draft.host = "127.0.0.1".into();
        draft.port = port.to_string();
        draft
    }

    #[test]
    fn shared_controller_keeps_ui_state_equal_to_committed_runtime_state() {
        let fail_apply = Arc::new(AtomicBool::new(false));
        let mut controller = ManagedDesktopController::new(
            AppConfig::default(),
            Runtime {
                fail_apply: Arc::clone(&fail_apply),
            },
            Store::default(),
            MemoryCredentialVault::default(),
        )
        .unwrap();
        let a = controller.save_proxy(draft("A", 1080), false).unwrap();
        let b = controller.save_proxy(draft("B", 1081), false).unwrap();
        controller.select_proxy(&a, false).unwrap();
        assert_eq!(controller.state().config.profiles.active_id(), Some(&a));

        controller.switch_mode(RoutingMode::Rules, false).unwrap();
        assert_eq!(
            controller.state().runtime.applied_mode,
            Some(RoutingMode::Rules)
        );

        assert!(matches!(
            controller.select_proxy(&b, false),
            Err(UiControlError::ConfirmationRequired(_))
        ));
        assert_eq!(controller.state().config.profiles.active_id(), Some(&a));
        controller.select_proxy(&b, true).unwrap();
        assert_eq!(controller.state().config.profiles.active_id(), Some(&b));

        let original = controller.state().config.profiles.get(&b).unwrap().clone();
        let mut edited = ProxyDraft::from_profile(&original);
        edited.port = "1181".into();
        assert!(matches!(
            controller.save_proxy(edited.clone(), false),
            Err(UiControlError::ConfirmationRequired(_))
        ));
        assert_eq!(
            controller.state().config.profiles.get(&b).unwrap().port,
            1081
        );
        controller.save_proxy(edited, true).unwrap();
        assert_eq!(
            controller.state().config.profiles.get(&b).unwrap().port,
            1181
        );

        fail_apply.store(true, Ordering::SeqCst);
        assert!(
            controller
                .switch_mode(RoutingMode::GlobalProxy, true)
                .is_err()
        );
        let state = controller.state();
        assert_eq!(state.runtime.applied_mode, Some(RoutingMode::Rules));
        assert_eq!(state.config.last_applied_mode, RoutingMode::Rules);
        assert!(state.runtime.failure.is_some());
        assert!(controller.delete_proxy(&b).is_err());
        assert_eq!(controller.state().config.profiles.active_id(), Some(&b));
    }

    #[test]
    fn proxy_draft_reports_fields_and_never_places_password_in_config() {
        let mut controller = ManagedDesktopController::new(
            AppConfig::default(),
            Runtime {
                fail_apply: Arc::new(AtomicBool::new(false)),
            },
            Store::default(),
            MemoryCredentialVault::default(),
        )
        .unwrap();
        let error = controller
            .save_proxy(ProxyDraft::default(), false)
            .unwrap_err();
        assert!(matches!(
            error,
            UiControlError::Validation { field: "name", .. }
        ));

        let mut authenticated = draft("Authenticated", 8080);
        authenticated.protocol = ProxyProtocol::Http;
        authenticated.auth_enabled = true;
        authenticated.username = "fixture-user".into();
        authenticated.password = "fixture-password".into();
        let id = controller.save_proxy(authenticated, false).unwrap();
        let profile = controller.state().config.profiles.get(&id).unwrap().clone();
        assert!(profile.auth_enabled);
        assert!(profile.credential_ref.is_some());
        let json = serde_json::to_string(&controller.state().config).unwrap();
        assert!(!json.contains("fixture-user"));
        assert!(!json.contains("fixture-password"));
    }

    #[test]
    fn rule_drafts_cover_all_targets_and_runtime_changes_require_confirmation() {
        let mut controller = ManagedDesktopController::new(
            AppConfig::default(),
            Runtime {
                fail_apply: Arc::new(AtomicBool::new(false)),
            },
            Store::default(),
            MemoryCredentialVault::default(),
        )
        .unwrap();
        let cases = [
            (RuleTargetKind::Domain, "example.com", "443"),
            (RuleTargetKind::DomainSuffix, "example.net", ""),
            (RuleTargetKind::Ip, "203.0.113.10", "22,443,8000-9000"),
            (RuleTargetKind::Cidr, "2001:db8::/32", "53"),
            (RuleTargetKind::Range, "192.0.2.10–192.0.2.30", "80"),
        ];
        let mut ids = Vec::new();
        for (index, (target_kind, target, ports)) in cases.into_iter().enumerate() {
            let draft = RuleDraft {
                name: format!("rule-{index}"),
                target_kind,
                target: target.into(),
                ports: ports.into(),
                ..RuleDraft::default()
            };
            ids.push(controller.save_rule(draft, false).unwrap());
        }
        assert_eq!(controller.state().config.rules.len(), 5);
        assert_eq!(
            controller.state().config.rules[2].ports.intervals(),
            &[(22, 22), (443, 443), (8000, 9000)]
        );

        let proxy = controller.save_proxy(draft("A", 1080), false).unwrap();
        controller.select_proxy(&proxy, false).unwrap();
        controller.switch_mode(RoutingMode::Rules, true).unwrap();
        assert!(matches!(
            controller.set_rule_enabled(&ids[0], false, false),
            Err(UiControlError::ConfirmationRequired(_))
        ));
        assert!(controller.state().config.rules[0].enabled);
        controller.set_rule_enabled(&ids[0], false, true).unwrap();
        assert!(!controller.state().config.rules[0].enabled);
        assert!(matches!(
            controller.delete_rule(&ids[1], false),
            Err(UiControlError::ConfirmationRequired(_))
        ));
        assert_eq!(controller.state().config.rules.len(), 5);
        controller.delete_rule(&ids[1], true).unwrap();
        assert_eq!(controller.state().config.rules.len(), 4);
    }

    #[test]
    fn rule_draft_reports_the_specific_invalid_field() {
        let mut controller = ManagedDesktopController::new(
            AppConfig::default(),
            Runtime {
                fail_apply: Arc::new(AtomicBool::new(false)),
            },
            Store::default(),
            MemoryCredentialVault::default(),
        )
        .unwrap();
        let error = controller
            .save_rule(RuleDraft::default(), false)
            .unwrap_err();
        assert!(matches!(
            error,
            UiControlError::Validation { field: "name", .. }
        ));
        let invalid = RuleDraft {
            name: "invalid target".into(),
            target: "https://example.com/path".into(),
            ..RuleDraft::default()
        };
        let error = controller.save_rule(invalid, false).unwrap_err();
        assert!(matches!(
            error,
            UiControlError::Validation {
                field: "target",
                ..
            }
        ));
    }

    #[test]
    fn backup_round_trip_is_secret_free_and_reports_missing_credentials() {
        let mut source = ManagedDesktopController::new(
            AppConfig::default(),
            Runtime {
                fail_apply: Arc::new(AtomicBool::new(false)),
            },
            Store::default(),
            MemoryCredentialVault::default(),
        )
        .unwrap();
        let mut authenticated = draft("Authenticated", 8080);
        authenticated.auth_enabled = true;
        authenticated.username = "fixture-user".into();
        authenticated.password = "fixture-password".into();
        source.save_proxy(authenticated, false).unwrap();
        source
            .save_rule(
                RuleDraft {
                    name: "HTTPS".into(),
                    target: "example.com".into(),
                    ports: "443".into(),
                    ..RuleDraft::default()
                },
                true,
            )
            .unwrap();

        let bytes = source.export_backup().unwrap();
        let json = String::from_utf8(bytes.clone()).unwrap();
        assert!(!json.contains("fixture-user"));
        assert!(!json.contains("fixture-password"));
        assert!(!json.contains("credential_ref"));
        assert!(!json.contains("last_applied_mode"));
        assert_eq!(
            source.preview_backup(&bytes).unwrap(),
            BackupPreview {
                profiles: 1,
                rules: 1,
                missing_credentials: 1,
            }
        );

        let mut target = ManagedDesktopController::new(
            AppConfig::default(),
            Runtime {
                fail_apply: Arc::new(AtomicBool::new(false)),
            },
            Store::default(),
            MemoryCredentialVault::default(),
        )
        .unwrap();
        target.import_backup(&bytes).unwrap();
        let state = target.state();
        assert_eq!(state.config.profiles.iter().count(), 1);
        assert_eq!(state.config.rules.len(), 1);
        assert_eq!(state.runtime.applied_mode, Some(RoutingMode::Direct));
        assert_eq!(state.config.last_applied_mode, RoutingMode::Direct);
        let before_clear = target.state();
        target.clear_logs().unwrap();
        let after_clear = target.state();
        assert_eq!(after_clear.config, before_clear.config);
        assert_eq!(after_clear.runtime, before_clear.runtime);
    }

    #[test]
    fn backup_import_rejects_proxy_mode_and_invalid_data_without_changing_state() {
        let mut controller = ManagedDesktopController::new(
            AppConfig::default(),
            Runtime {
                fail_apply: Arc::new(AtomicBool::new(false)),
            },
            Store::default(),
            MemoryCredentialVault::default(),
        )
        .unwrap();
        let id = controller
            .save_proxy(draft("Current", 1080), false)
            .unwrap();
        controller.select_proxy(&id, false).unwrap();
        let valid = controller.export_backup().unwrap();
        controller.switch_mode(RoutingMode::Rules, true).unwrap();
        let before = controller.state().config;
        assert!(controller.import_backup(&valid).is_err());
        assert_eq!(controller.state().config, before);

        controller.switch_mode(RoutingMode::Direct, true).unwrap();
        let before = controller.state().config;
        assert!(controller.import_backup(b"not json").is_err());
        assert_eq!(controller.state().config, before);
    }

    struct ExitingRuntime {
        exited: Arc<AtomicBool>,
    }

    impl ApplicationRuntime for ExitingRuntime {
        type Error = io::Error;

        fn plan(
            &self,
            _current: &AppConfig,
            current_mode: RoutingMode,
            _candidate: &AppConfig,
            candidate_mode: RoutingMode,
        ) -> ApplyPlan {
            if current_mode == candidate_mode {
                ApplyPlan::Hot
            } else {
                ApplyPlan::Restart
            }
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

        fn poll_failure(&mut self) -> Result<Option<String>, String> {
            Ok(self
                .exited
                .load(Ordering::SeqCst)
                .then(|| "内核意外退出".into()))
        }
    }

    #[test]
    fn shared_state_observes_runtime_exit_and_keeps_last_successful_mode() {
        let exited = Arc::new(AtomicBool::new(false));
        let mut controller = ManagedDesktopController::new(
            AppConfig::default(),
            ExitingRuntime {
                exited: Arc::clone(&exited),
            },
            Store::default(),
            MemoryCredentialVault::default(),
        )
        .unwrap();
        let proxy = controller.save_proxy(draft("A", 1080), false).unwrap();
        controller.select_proxy(&proxy, false).unwrap();
        controller.switch_mode(RoutingMode::Rules, true).unwrap();
        exited.store(true, Ordering::SeqCst);

        let failed = controller.state();
        assert_eq!(failed.runtime.phase, crate::core::SwitchPhase::Error);
        assert_eq!(failed.runtime.applied_mode, None);
        assert!(failed.runtime.traffic_may_be_direct);
        assert_eq!(failed.config.last_applied_mode, RoutingMode::Rules);

        controller.switch_mode(RoutingMode::Direct, true).unwrap();
        let direct = controller.state();
        assert_eq!(direct.runtime.applied_mode, Some(RoutingMode::Direct));
        assert!(!direct.runtime.traffic_may_be_direct);
    }
}
