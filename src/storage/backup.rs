use super::{AppConfig, ConfigError, ConfigStore, Preferences};
use crate::{
    domain::{ProfileId, ProxyHost, ProxyProfile, ProxyProfiles, ProxyProtocol, RoutingRule},
    routing::RoutingMode,
};
use serde::{Deserialize, Serialize};
use std::fmt;

const BACKUP_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupFile {
    schema_version: u32,
    profiles: Vec<BackupProfile>,
    active_profile_id: Option<ProfileId>,
    rules: Vec<RoutingRule>,
    preferences: Preferences,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupProfile {
    id: ProfileId,
    name: String,
    protocol: ProxyProtocol,
    host: ProxyHost,
    port: u16,
    auth_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportRuntimeState {
    pub applied_mode: RoutingMode,
    pub network_restored: bool,
    pub transition_in_progress: bool,
    pub config_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPreview {
    replacement: AppConfig,
    expected_revision: u64,
}

impl ImportPreview {
    pub fn replacement(&self) -> &AppConfig {
        &self.replacement
    }
}

#[derive(Debug)]
pub enum ImportError {
    InvalidJson(serde_json::Error),
    UnsupportedVersion(u32),
    InvalidConfig(ConfigError),
    NetworkNotRestored,
    RuntimeChanged,
    RevisionChanged,
    Save(ConfigError),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(f, "备份 JSON 无效: {error}"),
            Self::UnsupportedVersion(version) => write!(f, "不支持备份版本 {version}"),
            Self::InvalidConfig(error) => write!(f, "备份内容无效: {error}"),
            Self::NetworkNotRestored => f.write_str("仅可在网络已恢复的全局直连状态导入"),
            Self::RuntimeChanged => f.write_str("预览后运行状态已变化，请重新预览"),
            Self::RevisionChanged => f.write_str("预览后配置已变化，请重新预览"),
            Self::Save(error) => write!(f, "导入保存失败: {error}"),
        }
    }
}

impl std::error::Error for ImportError {}

pub fn export_backup(config: &AppConfig) -> Result<Vec<u8>, ConfigError> {
    config.validate()?;
    let backup = BackupFile {
        schema_version: BACKUP_SCHEMA_VERSION,
        profiles: config
            .profiles
            .iter()
            .map(|profile| BackupProfile {
                id: profile.id.clone(),
                name: profile.name.clone(),
                protocol: profile.protocol,
                host: profile.host.clone(),
                port: profile.port,
                auth_enabled: profile.auth_enabled,
            })
            .collect(),
        active_profile_id: config.profiles.active_id().cloned(),
        rules: config.rules.clone(),
        preferences: config.preferences.clone(),
    };
    serde_json::to_vec_pretty(&backup).map_err(ConfigError::InvalidJson)
}

pub fn preview_import(bytes: &[u8], current_revision: u64) -> Result<ImportPreview, ImportError> {
    let backup: BackupFile = serde_json::from_slice(bytes).map_err(ImportError::InvalidJson)?;
    if backup.schema_version != BACKUP_SCHEMA_VERSION {
        return Err(ImportError::UnsupportedVersion(backup.schema_version));
    }

    let mut profiles = ProxyProfiles::default();
    for profile in backup.profiles {
        profiles
            .create(ProxyProfile {
                id: profile.id,
                name: profile.name,
                protocol: profile.protocol,
                host: profile.host,
                port: profile.port,
                auth_enabled: profile.auth_enabled,
                credential_ref: None,
            })
            .map_err(|error| ImportError::InvalidConfig(ConfigError::Validation(error)))?;
    }
    if let Some(active) = backup.active_profile_id {
        profiles
            .select(&active)
            .map_err(|error| ImportError::InvalidConfig(ConfigError::Validation(error)))?;
    }
    let replacement = AppConfig {
        revision: current_revision.saturating_add(1),
        profiles,
        rules: backup.rules,
        last_applied_mode: RoutingMode::Direct,
        preferences: backup.preferences,
        ..AppConfig::default()
    };
    replacement.validate().map_err(ImportError::InvalidConfig)?;
    Ok(ImportPreview {
        replacement,
        expected_revision: current_revision,
    })
}

pub fn commit_import(
    preview: ImportPreview,
    runtime: ImportRuntimeState,
    store: &ConfigStore,
) -> Result<AppConfig, ImportError> {
    if runtime.transition_in_progress {
        return Err(ImportError::RuntimeChanged);
    }
    if runtime.applied_mode != RoutingMode::Direct || !runtime.network_restored {
        return Err(ImportError::NetworkNotRestored);
    }
    if runtime.config_revision != preview.expected_revision {
        return Err(ImportError::RevisionChanged);
    }
    store
        .save(&preview.replacement)
        .map_err(ImportError::Save)?;
    Ok(preview.replacement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CredentialRef, ProfileId};
    use std::{
        fs, io,
        path::{Path, PathBuf},
    };
    use uuid::Uuid;

    struct FailingWriter;

    impl super::super::AtomicWriter for FailingWriter {
        fn write(&self, _path: &Path, _bytes: &[u8]) -> io::Result<()> {
            Err(io::Error::other("injected import failure"))
        }
    }

    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("socks-proxy-import-{name}-{}", Uuid::new_v4()))
    }

    fn authenticated_config() -> AppConfig {
        let mut profiles = ProxyProfiles::default();
        let profile = ProxyProfile {
            id: ProfileId::new(),
            name: "authenticated".into(),
            protocol: ProxyProtocol::Http,
            host: ProxyHost::parse("proxy.example.com").unwrap(),
            port: 8080,
            auth_enabled: true,
            credential_ref: Some(CredentialRef::parse("secret-reference-9").unwrap()),
        };
        let id = profile.id.clone();
        profiles.create(profile).unwrap();
        profiles.select(&id).unwrap();
        AppConfig {
            revision: 7,
            profiles,
            last_applied_mode: RoutingMode::Rules,
            ..AppConfig::default()
        }
    }

    #[test]
    fn export_contains_auth_marker_but_no_secret_or_runtime_mode() {
        let bytes = export_backup(&authenticated_config()).unwrap();
        let json = String::from_utf8(bytes).unwrap();
        assert!(json.contains("auth_enabled"));
        assert!(json.contains("true"));
        assert!(!json.contains("secret-reference-9"));
        assert!(!json.contains("credential_ref"));
        assert!(!json.contains("last_applied_mode"));
    }

    #[test]
    fn preview_marks_credentials_missing_and_forces_direct() {
        let bytes = export_backup(&authenticated_config()).unwrap();
        let preview = preview_import(&bytes, 12).unwrap();
        assert_eq!(preview.replacement().revision, 13);
        assert_eq!(preview.replacement().last_applied_mode, RoutingMode::Direct);
        let active = preview.replacement().profiles.active().unwrap();
        assert!(active.auth_enabled);
        assert!(active.credential_ref.is_none());
        assert!(preview.replacement().profiles.require_active().is_err());
    }

    #[test]
    fn commit_rechecks_direct_recovery_transition_and_revision() {
        let bytes = export_backup(&authenticated_config()).unwrap();
        let root = path("state");
        let store = ConfigStore::new(root.join("config.json"));
        let valid = ImportRuntimeState {
            applied_mode: RoutingMode::Direct,
            network_restored: true,
            transition_in_progress: false,
            config_revision: 7,
        };
        for invalid in [
            ImportRuntimeState {
                applied_mode: RoutingMode::Rules,
                ..valid
            },
            ImportRuntimeState {
                network_restored: false,
                ..valid
            },
            ImportRuntimeState {
                transition_in_progress: true,
                ..valid
            },
            ImportRuntimeState {
                config_revision: 8,
                ..valid
            },
        ] {
            assert!(commit_import(preview_import(&bytes, 7).unwrap(), invalid, &store).is_err());
            assert!(!root.join("config.json").exists());
        }
        let imported = commit_import(preview_import(&bytes, 7).unwrap(), valid, &store).unwrap();
        assert_eq!(store.load().unwrap(), imported);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_unknown_and_invalid_backups_are_rejected() {
        assert!(preview_import(b"not json", 0).is_err());
        let valid = export_backup(&AppConfig::default()).unwrap();
        let unknown = String::from_utf8(valid.clone()).unwrap().replacen(
            "\"schema_version\": 1",
            "\"schema_version\": 999",
            1,
        );
        assert!(matches!(
            preview_import(unknown.as_bytes(), 0),
            Err(ImportError::UnsupportedVersion(999))
        ));
        let dangling = String::from_utf8(valid).unwrap().replacen(
            "\"active_profile_id\": null",
            "\"active_profile_id\": \"00000000-0000-4000-8000-000000000001\"",
            1,
        );
        assert!(preview_import(dangling.as_bytes(), 0).is_err());
    }

    #[test]
    fn commit_write_failure_preserves_the_old_configuration() {
        let root = path("write-failure");
        let path = root.join("config.json");
        let old = AppConfig::default();
        ConfigStore::new(&path).save(&old).unwrap();
        let bytes = export_backup(&authenticated_config()).unwrap();
        let preview = preview_import(&bytes, 0).unwrap();
        let failing = ConfigStore::with_writer(&path, Box::new(FailingWriter));
        let runtime = ImportRuntimeState {
            applied_mode: RoutingMode::Direct,
            network_restored: true,
            transition_in_progress: false,
            config_revision: 0,
        };
        assert!(matches!(
            commit_import(preview, runtime, &failing),
            Err(ImportError::Save(_))
        ));
        assert_eq!(ConfigStore::new(&path).load().unwrap(), old);
        let _ = fs::remove_dir_all(root);
    }
}
