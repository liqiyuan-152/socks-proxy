use crate::{
    domain::{ProxyProfiles, RoutingRule, ValidationError},
    routing::RoutingMode,
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fmt,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub start_with_windows: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    pub revision: u64,
    pub cache_initialized: bool,
    pub profiles: ProxyProfiles,
    pub rules: Vec<RoutingRule>,
    pub last_applied_mode: RoutingMode,
    pub preferences: Preferences,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            revision: 0,
            cache_initialized: false,
            profiles: ProxyProfiles::default(),
            rules: Vec::new(),
            last_applied_mode: RoutingMode::Direct,
            preferences: Preferences::default(),
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedVersion(self.schema_version));
        }
        self.profiles.validate().map_err(ConfigError::Validation)?;
        for rule in &self.rules {
            rule.validate().map_err(ConfigError::Validation)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    InvalidJson(serde_json::Error),
    UnsupportedVersion(u32),
    Validation(ValidationError),
    PrimaryAndBackupInvalid {
        primary: Box<ConfigError>,
        backup: Box<ConfigError>,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "配置文件读写失败: {error}"),
            Self::InvalidJson(error) => write!(f, "配置 JSON 无效: {error}"),
            Self::UnsupportedVersion(version) => write!(f, "不支持配置版本 {version}"),
            Self::Validation(error) => write!(f, "配置校验失败: {error}"),
            Self::PrimaryAndBackupInvalid { primary, backup } => {
                write!(f, "主配置无效 ({primary})，备份也无效 ({backup})")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<io::Error> for ConfigError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

trait AtomicWriter: Send + Sync {
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
}

struct FsAtomicWriter;

impl AtomicWriter for FsAtomicWriter {
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "配置路径缺少父目录"))?;
        fs::create_dir_all(parent)?;
        let temp_path = parent.join(format!(".config-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut temp = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)?;
            temp.write_all(bytes)?;
            temp.sync_all()?;
            drop(temp);
            replace_file(&temp_path, path)?;
            sync_parent(parent)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        result
    }
}

pub struct ConfigStore {
    path: PathBuf,
    writer: Box<dyn AtomicWriter>,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            writer: Box::new(FsAtomicWriter),
        }
    }

    pub fn load(&self) -> Result<AppConfig, ConfigError> {
        if !self.path.exists() {
            let backup = backup_path(&self.path);
            return if backup.exists() {
                read_config(&backup)
            } else {
                Ok(AppConfig::default())
            };
        }
        match read_config(&self.path) {
            Ok(config) => Ok(config),
            Err(primary) => {
                let backup = backup_path(&self.path);
                match read_config(&backup) {
                    Ok(config) => Ok(config),
                    Err(backup) => Err(ConfigError::PrimaryAndBackupInvalid {
                        primary: Box::new(primary),
                        backup: Box::new(backup),
                    }),
                }
            }
        }
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), ConfigError> {
        config.validate()?;
        let bytes = serde_json::to_vec_pretty(config).map_err(ConfigError::InvalidJson)?;
        if self.path.exists() && read_config(&self.path).is_ok() {
            let previous = fs::read(&self.path)?;
            self.writer.write(&backup_path(&self.path), &previous)?;
        }
        self.writer.write(&self.path, &bytes)?;
        Ok(())
    }

    #[cfg(test)]
    fn with_writer(path: impl Into<PathBuf>, writer: Box<dyn AtomicWriter>) -> Self {
        Self {
            path: path.into(),
            writer,
        }
    }
}

fn read_config(path: &Path) -> Result<AppConfig, ConfigError> {
    let bytes = fs::read(path)?;
    let config = serde_json::from_slice(&bytes).map_err(ConfigError::InvalidJson)?;
    AppConfig::validate(&config)?;
    Ok(config)
}

fn backup_path(path: &Path) -> PathBuf {
    let mut file_name: OsString = path.file_name().unwrap_or(path.as_os_str()).to_owned();
    file_name.push(".bak");
    path.with_file_name(file_name)
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let succeeded = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> io::Result<()> {
    fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("socks-proxy-{name}-{}", Uuid::new_v4()))
    }

    struct FailOnCall {
        call: AtomicUsize,
        fail_on: usize,
    }

    impl AtomicWriter for FailOnCall {
        fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            let call = self.call.fetch_add(1, Ordering::SeqCst) + 1;
            if call == self.fail_on {
                return Err(io::Error::other("injected write failure"));
            }
            FsAtomicWriter.write(path, bytes)
        }
    }

    #[test]
    fn first_launch_is_direct_and_startup_is_disabled() {
        let root = test_path("first-launch");
        let store = ConfigStore::new(root.join("config.json"));
        let config = store.load().unwrap();
        assert_eq!(config.last_applied_mode, RoutingMode::Direct);
        assert!(!config.preferences.start_with_windows);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn restart_restores_last_successfully_saved_mode() {
        let root = test_path("restart");
        let path = root.join("config.json");
        let store = ConfigStore::new(&path);
        let config = AppConfig {
            last_applied_mode: RoutingMode::Rules,
            ..AppConfig::default()
        };
        store.save(&config).unwrap();
        assert_eq!(ConfigStore::new(&path).load().unwrap(), config);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_replacement_keeps_old_config_and_valid_backup() {
        let root = test_path("failed-save");
        let path = root.join("config.json");
        let initial = AppConfig::default();
        ConfigStore::new(&path).save(&initial).unwrap();

        let failing = ConfigStore::with_writer(
            &path,
            Box::new(FailOnCall {
                call: AtomicUsize::new(0),
                fail_on: 2,
            }),
        );
        let changed = AppConfig {
            last_applied_mode: RoutingMode::GlobalProxy,
            ..AppConfig::default()
        };
        assert!(failing.save(&changed).is_err());
        assert_eq!(ConfigStore::new(&path).load().unwrap(), initial);
        assert_eq!(read_config(&backup_path(&path)).unwrap(), initial);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupted_primary_recovers_the_last_valid_backup() {
        let root = test_path("backup");
        let path = root.join("config.json");
        let store = ConfigStore::new(&path);
        let first = AppConfig::default();
        store.save(&first).unwrap();
        let second = AppConfig {
            last_applied_mode: RoutingMode::Rules,
            ..AppConfig::default()
        };
        store.save(&second).unwrap();
        fs::write(&path, b"not json").unwrap();
        assert_eq!(store.load().unwrap(), first);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cache_initialization_marker_is_persisted() {
        let root = test_path("cache-marker");
        let path = root.join("config.json");
        let config = AppConfig {
            cache_initialized: true,
            ..AppConfig::default()
        };
        ConfigStore::new(&path).save(&config).unwrap();
        assert!(ConfigStore::new(&path).load().unwrap().cache_initialized);
        let _ = fs::remove_dir_all(root);
    }
}
mod backup;
mod transaction;

pub use backup::{
    ImportError, ImportPreview, ImportRuntimeState, commit_import, export_backup, preview_import,
};
pub use transaction::{
    CredentialVault, FileTransactionJournal, RuntimeApplier, TransactionError, TransactionJournal,
    TransactionRecord, TransactionStage, recover_transaction, referenced_credentials,
    update_active_credential,
};
