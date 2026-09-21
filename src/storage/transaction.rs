use super::{AppConfig, AtomicWriter, ConfigError, ConfigStore};
use crate::domain::{CredentialRef, ProfileId, ValidationError};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt, fs, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionStage {
    Prepared,
    RuntimeApplied,
    ConfigCommitted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionRecord {
    pub old_config: AppConfig,
    pub new_config: AppConfig,
    pub stage: TransactionStage,
}

impl TransactionRecord {
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.old_config.validate()?;
        self.new_config.validate()?;
        if self.new_config.revision != self.old_config.revision.saturating_add(1) {
            return Err(ConfigError::Validation(ValidationError(
                "事务配置修订不连续",
            )));
        }
        Ok(())
    }
}

pub trait CredentialVault {
    type Error: std::error::Error + Send + Sync + 'static;

    fn create_version(&mut self, secret: &[u8]) -> Result<CredentialRef, Self::Error>;
    fn delete_version(&mut self, reference: &CredentialRef) -> Result<(), Self::Error>;
}

pub trait RuntimeApplier {
    type Error: std::error::Error + Send + Sync + 'static;

    fn apply(&mut self, config: &AppConfig) -> Result<(), Self::Error>;
}

pub trait TransactionJournal {
    type Error: std::error::Error + Send + Sync + 'static;

    fn save(&mut self, record: &TransactionRecord) -> Result<(), Self::Error>;
    fn clear(&mut self) -> Result<(), Self::Error>;
}

pub struct FileTransactionJournal {
    path: PathBuf,
}

impl FileTransactionJournal {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> Result<Option<TransactionRecord>, ConfigError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&self.path)?;
        let record: TransactionRecord =
            serde_json::from_slice(&bytes).map_err(ConfigError::InvalidJson)?;
        record.validate()?;
        Ok(Some(record))
    }
}

impl TransactionJournal for FileTransactionJournal {
    type Error = ConfigError;

    fn save(&mut self, record: &TransactionRecord) -> Result<(), Self::Error> {
        record.validate()?;
        let bytes = serde_json::to_vec_pretty(record).map_err(ConfigError::InvalidJson)?;
        super::FsAtomicWriter.write(&self.path, &bytes)?;
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ConfigError::Io(error)),
        }
    }
}

#[derive(Debug)]
pub enum TransactionError {
    ProfileMissing,
    Credential(String),
    Journal(String),
    Runtime(String),
    Config(ConfigError),
    Rollback { cause: String, rollback: String },
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProfileMissing => f.write_str("待修改的代理不存在"),
            Self::Credential(error) => write!(f, "凭据写入失败: {error}"),
            Self::Journal(error) => write!(f, "事务记录失败: {error}"),
            Self::Runtime(error) => write!(f, "内核应用失败: {error}"),
            Self::Config(error) => write!(f, "配置提交失败: {error}"),
            Self::Rollback { cause, rollback } => {
                write!(f, "事务失败 ({cause})，且回滚失败 ({rollback})")
            }
        }
    }
}

impl std::error::Error for TransactionError {}

pub fn update_active_credential<V, R, J>(
    current: &AppConfig,
    profile_id: &ProfileId,
    secret: &[u8],
    store: &ConfigStore,
    vault: &mut V,
    runtime: &mut R,
    journal: &mut J,
) -> Result<AppConfig, TransactionError>
where
    V: CredentialVault,
    R: RuntimeApplier,
    J: TransactionJournal,
{
    let existing_profile = current
        .profiles
        .get(profile_id)
        .cloned()
        .ok_or(TransactionError::ProfileMissing)?;
    let new_reference = vault
        .create_version(secret)
        .map_err(|error| TransactionError::Credential(error.to_string()))?;
    let mut replacement = current.clone();
    let mut profile = existing_profile;
    profile.auth_enabled = true;
    profile.credential_ref = Some(new_reference.clone());
    replacement
        .profiles
        .update(profile)
        .map_err(|error| TransactionError::Config(ConfigError::Validation(error)))?;
    replacement.revision = current.revision.saturating_add(1);

    let mut record = TransactionRecord {
        old_config: current.clone(),
        new_config: replacement.clone(),
        stage: TransactionStage::Prepared,
    };
    if let Err(error) = journal.save(&record) {
        let _ = vault.delete_version(&new_reference);
        return Err(TransactionError::Journal(error.to_string()));
    }

    if let Err(error) = runtime.apply(&replacement) {
        let cause = error.to_string();
        if let Err(rollback) = runtime.apply(current) {
            return Err(TransactionError::Rollback {
                cause,
                rollback: rollback.to_string(),
            });
        }
        let _ = vault.delete_version(&new_reference);
        let _ = journal.clear();
        return Err(TransactionError::Runtime(cause));
    }

    record.stage = TransactionStage::RuntimeApplied;
    if let Err(error) = journal.save(&record) {
        return rollback_runtime(current, runtime, error.to_string());
    }
    if let Err(error) = store.save(&replacement) {
        let cause = error.to_string();
        if let Err(rollback) = runtime.apply(current) {
            return Err(TransactionError::Rollback {
                cause,
                rollback: rollback.to_string(),
            });
        }
        let _ = vault.delete_version(&new_reference);
        let _ = journal.clear();
        return Err(TransactionError::Config(error));
    }

    record.stage = TransactionStage::ConfigCommitted;
    journal
        .save(&record)
        .map_err(|error| TransactionError::Journal(error.to_string()))?;
    journal
        .clear()
        .map_err(|error| TransactionError::Journal(error.to_string()))?;
    Ok(replacement)
}

fn rollback_runtime<R: RuntimeApplier>(
    old_config: &AppConfig,
    runtime: &mut R,
    cause: String,
) -> Result<AppConfig, TransactionError> {
    match runtime.apply(old_config) {
        Ok(()) => Err(TransactionError::Journal(cause)),
        Err(rollback) => Err(TransactionError::Rollback {
            cause,
            rollback: rollback.to_string(),
        }),
    }
}

pub fn recover_transaction<R, J>(
    record: &TransactionRecord,
    persisted: &AppConfig,
    runtime: &mut R,
    journal: &mut J,
) -> Result<AppConfig, TransactionError>
where
    R: RuntimeApplier,
    J: TransactionJournal,
{
    record.validate().map_err(TransactionError::Config)?;
    let selected = if persisted.revision == record.new_config.revision {
        &record.new_config
    } else if persisted.revision == record.old_config.revision {
        &record.old_config
    } else {
        return Err(TransactionError::Config(ConfigError::Validation(
            ValidationError("磁盘配置修订与恢复记录不一致"),
        )));
    };
    runtime
        .apply(selected)
        .map_err(|error| TransactionError::Runtime(error.to_string()))?;
    journal
        .clear()
        .map_err(|error| TransactionError::Journal(error.to_string()))?;
    Ok(selected.clone())
}

pub fn referenced_credentials(configs: &[&AppConfig]) -> BTreeSet<String> {
    configs
        .iter()
        .flat_map(|config| config.profiles.iter())
        .filter_map(|profile| profile.credential_ref.as_ref())
        .map(|reference| reference.as_str().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ProfileId, ProxyHost, ProxyProfile, ProxyProfiles, ProxyProtocol};
    use std::{
        collections::BTreeMap,
        io,
        path::{Path, PathBuf},
    };
    use uuid::Uuid;

    #[derive(Default)]
    struct Vault {
        fail_create: bool,
        entries: BTreeMap<String, Vec<u8>>,
    }

    impl CredentialVault for Vault {
        type Error = io::Error;
        fn create_version(&mut self, secret: &[u8]) -> Result<CredentialRef, Self::Error> {
            if self.fail_create {
                return Err(io::Error::other("create failed"));
            }
            let reference =
                CredentialRef::parse(&format!("credential-{}", Uuid::new_v4())).unwrap();
            self.entries
                .insert(reference.as_str().into(), secret.to_vec());
            Ok(reference)
        }
        fn delete_version(&mut self, reference: &CredentialRef) -> Result<(), Self::Error> {
            self.entries.remove(reference.as_str());
            Ok(())
        }
    }

    #[derive(Default)]
    struct Runtime {
        fail_at: Option<usize>,
        calls: usize,
        revisions: Vec<u64>,
    }

    impl RuntimeApplier for Runtime {
        type Error = io::Error;
        fn apply(&mut self, config: &AppConfig) -> Result<(), Self::Error> {
            self.calls += 1;
            if self.fail_at == Some(self.calls) {
                return Err(io::Error::other("apply failed"));
            }
            self.revisions.push(config.revision);
            Ok(())
        }
    }

    #[derive(Default)]
    struct Journal {
        fail_at: Option<usize>,
        calls: usize,
        record: Option<TransactionRecord>,
    }

    impl TransactionJournal for Journal {
        type Error = io::Error;
        fn save(&mut self, record: &TransactionRecord) -> Result<(), Self::Error> {
            self.calls += 1;
            if self.fail_at == Some(self.calls) {
                return Err(io::Error::other("journal failed"));
            }
            self.record = Some(record.clone());
            Ok(())
        }
        fn clear(&mut self) -> Result<(), Self::Error> {
            self.record = None;
            Ok(())
        }
    }

    struct FailingWriter;

    impl super::super::AtomicWriter for FailingWriter {
        fn write(&self, _path: &Path, _bytes: &[u8]) -> io::Result<()> {
            Err(io::Error::other("config save failed"))
        }
    }

    fn config() -> (AppConfig, ProfileId) {
        let old_ref = CredentialRef::parse("credential-old").unwrap();
        let profile = ProxyProfile {
            id: ProfileId::new(),
            name: "active".into(),
            protocol: ProxyProtocol::Socks5,
            host: ProxyHost::parse("127.0.0.1").unwrap(),
            port: 1080,
            auth_enabled: true,
            credential_ref: Some(old_ref),
        };
        let id = profile.id.clone();
        let mut profiles = ProxyProfiles::default();
        profiles.create(profile).unwrap();
        profiles.select(&id).unwrap();
        (
            AppConfig {
                revision: 4,
                profiles,
                ..AppConfig::default()
            },
            id,
        )
    }

    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("socks-proxy-transaction-{name}-{}", Uuid::new_v4()))
    }

    #[test]
    fn credential_failure_leaves_config_and_runtime_untouched() {
        let (old, id) = config();
        let mut vault = Vault {
            fail_create: true,
            ..Vault::default()
        };
        let mut runtime = Runtime::default();
        let mut journal = Journal::default();
        let result = update_active_credential(
            &old,
            &id,
            b"new-secret",
            &ConfigStore::new(path("credential")),
            &mut vault,
            &mut runtime,
            &mut journal,
        );
        assert!(matches!(result, Err(TransactionError::Credential(_))));
        assert!(runtime.revisions.is_empty());
        assert!(journal.record.is_none());
    }

    #[test]
    fn missing_profile_does_not_create_a_credential() {
        let (old, _) = config();
        let mut vault = Vault::default();
        let mut runtime = Runtime::default();
        let mut journal = Journal::default();
        let result = update_active_credential(
            &old,
            &ProfileId::new(),
            b"new-secret",
            &ConfigStore::new(path("missing-profile")),
            &mut vault,
            &mut runtime,
            &mut journal,
        );
        assert!(matches!(result, Err(TransactionError::ProfileMissing)));
        assert!(vault.entries.is_empty());
    }

    #[test]
    fn runtime_failure_rolls_back_to_old_revision() {
        let (old, id) = config();
        let mut vault = Vault::default();
        let mut runtime = Runtime {
            fail_at: Some(1),
            ..Runtime::default()
        };
        let mut journal = Journal::default();
        let result = update_active_credential(
            &old,
            &id,
            b"new-secret",
            &ConfigStore::new(path("runtime")),
            &mut vault,
            &mut runtime,
            &mut journal,
        );
        assert!(matches!(result, Err(TransactionError::Runtime(_))));
        assert_eq!(runtime.revisions, [4]);
        assert!(vault.entries.is_empty());
    }

    #[test]
    fn successful_commit_uses_an_immutable_new_reference() {
        let (old, id) = config();
        let root = path("success");
        let store = ConfigStore::new(root.join("config.json"));
        let mut vault = Vault::default();
        let mut runtime = Runtime::default();
        let mut journal = Journal::default();
        let committed = update_active_credential(
            &old,
            &id,
            b"new-secret",
            &store,
            &mut vault,
            &mut runtime,
            &mut journal,
        )
        .unwrap();
        assert_eq!(committed.revision, 5);
        assert_ne!(
            committed.profiles.get(&id).unwrap().credential_ref,
            old.profiles.get(&id).unwrap().credential_ref
        );
        assert_eq!(runtime.revisions, [5]);
        assert!(journal.record.is_none());
        assert_eq!(store.load().unwrap(), committed);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn config_failure_restores_old_runtime_and_preserves_old_disk_config() {
        let (old, id) = config();
        let root = path("config-failure");
        let path = root.join("config.json");
        ConfigStore::new(&path).save(&old).unwrap();
        let failing = ConfigStore::with_writer(&path, Box::new(FailingWriter));
        let mut vault = Vault::default();
        let mut runtime = Runtime::default();
        let mut journal = Journal::default();
        let result = update_active_credential(
            &old,
            &id,
            b"new-secret",
            &failing,
            &mut vault,
            &mut runtime,
            &mut journal,
        );
        assert!(matches!(result, Err(TransactionError::Config(_))));
        assert_eq!(runtime.revisions, [5, 4]);
        assert_eq!(ConfigStore::new(&path).load().unwrap(), old);
        assert!(journal.record.is_none());
        assert!(vault.entries.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn crash_recovery_follows_the_persisted_revision() {
        let (old, id) = config();
        let mut new = old.clone();
        new.revision = 5;
        let mut profile = new.profiles.get(&id).unwrap().clone();
        profile.credential_ref = Some(CredentialRef::parse("credential-new").unwrap());
        new.profiles.update(profile).unwrap();
        let record = TransactionRecord {
            old_config: old.clone(),
            new_config: new.clone(),
            stage: TransactionStage::RuntimeApplied,
        };
        let mut runtime = Runtime::default();
        let mut journal = Journal {
            record: Some(record.clone()),
            ..Journal::default()
        };
        assert_eq!(
            recover_transaction(&record, &old, &mut runtime, &mut journal)
                .unwrap()
                .revision,
            4
        );
        let mut journal = Journal {
            record: Some(record.clone()),
            ..Journal::default()
        };
        assert_eq!(
            recover_transaction(&record, &new, &mut runtime, &mut journal)
                .unwrap()
                .revision,
            5
        );
        assert_eq!(runtime.revisions, [4, 5]);
    }

    #[test]
    fn credential_gc_keeps_config_transaction_and_backup_references() {
        let (current, id) = config();
        let mut backup = current.clone();
        let mut profile = backup.profiles.get(&id).unwrap().clone();
        profile.credential_ref = Some(CredentialRef::parse("credential-backup").unwrap());
        backup.profiles.update(profile).unwrap();
        let refs = referenced_credentials(&[&current, &backup]);
        assert_eq!(
            refs,
            BTreeSet::from(["credential-backup".into(), "credential-old".into()])
        );
    }

    #[test]
    fn file_journal_is_atomic_loadable_and_contains_no_secret() {
        let (old, id) = config();
        let mut new = old.clone();
        new.revision = 5;
        let mut profile = new.profiles.get(&id).unwrap().clone();
        profile.credential_ref = Some(CredentialRef::parse("credential-new").unwrap());
        new.profiles.update(profile).unwrap();
        let record = TransactionRecord {
            old_config: old,
            new_config: new,
            stage: TransactionStage::Prepared,
        };
        let root = path("journal");
        let path = root.join("transaction.json");
        let mut journal = FileTransactionJournal::new(&path);
        journal.save(&record).unwrap();
        assert_eq!(journal.load().unwrap(), Some(record));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("new-secret"));
        journal.clear().unwrap();
        assert_eq!(journal.load().unwrap(), None);
        let _ = std::fs::remove_dir_all(root);
    }
}
