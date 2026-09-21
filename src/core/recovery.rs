use serde::{Deserialize, Serialize};
use std::{fmt, fs, io, path::PathBuf};

const RECOVERY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NetworkResource {
    DnsServers { interface_id: String },
    Route { destination: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkChange {
    pub resource: NetworkResource,
    pub value: Option<String>,
}

pub trait NetworkRecoveryBackend {
    type Error: std::error::Error + Send + Sync + 'static;

    fn read(&mut self, resource: &NetworkResource) -> Result<Option<String>, Self::Error>;
    fn write(&mut self, resource: &NetworkResource, value: Option<&str>)
    -> Result<(), Self::Error>;
    fn flush_dns_cache(&mut self) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct OwnedChange {
    resource: NetworkResource,
    original: Option<String>,
    applied: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RecoveryRecord {
    schema_version: u32,
    changes: Vec<OwnedChange>,
    dns_flush_pending: bool,
}

#[derive(Debug, Clone)]
pub struct RecoveryStore {
    path: PathBuf,
}

impl RecoveryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn load(&self) -> Result<Option<RecoveryRecord>, NetworkRecoveryError> {
        match fs::read(&self.path) {
            Ok(bytes) => {
                let record: RecoveryRecord = serde_json::from_slice(&bytes)?;
                if record.schema_version != RECOVERY_SCHEMA_VERSION {
                    return Err(NetworkRecoveryError::InvalidRecord(format!(
                        "不支持的网络恢复记录版本: {}",
                        record.schema_version
                    )));
                }
                Ok(Some(record))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn save(&self, record: &RecoveryRecord) -> Result<(), NetworkRecoveryError> {
        let parent = self.path.parent().ok_or_else(|| {
            NetworkRecoveryError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "网络恢复记录路径缺少父目录",
            ))
        })?;
        fs::create_dir_all(parent)?;
        let temporary = self.path.with_extension("tmp");
        let bytes = serde_json::to_vec_pretty(record)?;
        fs::write(&temporary, bytes)?;
        replace_file(&temporary, &self.path)?;
        Ok(())
    }

    fn clear(&self) -> Result<(), NetworkRecoveryError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkRecoveryReport {
    pub complete: bool,
    pub restored: Vec<NetworkResource>,
    pub preserved_external: Vec<NetworkResource>,
    pub pending: Vec<NetworkResource>,
    pub dns_flush_pending: bool,
    pub application_cache_notice: bool,
}

#[derive(Debug)]
pub enum NetworkRecoveryError {
    Io(io::Error),
    Json(serde_json::Error),
    InvalidRecord(String),
    Backend(String),
}

impl fmt::Display for NetworkRecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "网络恢复记录操作失败: {error}"),
            Self::Json(error) => write!(formatter, "网络恢复记录解析失败: {error}"),
            Self::InvalidRecord(error) | Self::Backend(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for NetworkRecoveryError {}

impl From<io::Error> for NetworkRecoveryError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for NetworkRecoveryError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub struct NetworkRecovery<B> {
    backend: B,
    store: RecoveryStore,
}

impl<B: NetworkRecoveryBackend> NetworkRecovery<B> {
    pub fn new(backend: B, store: RecoveryStore) -> Self {
        Self { backend, store }
    }

    pub fn apply(&mut self, changes: &[NetworkChange]) -> Result<(), NetworkRecoveryError> {
        let mut record = self.store.load()?.unwrap_or(RecoveryRecord {
            schema_version: RECOVERY_SCHEMA_VERSION,
            changes: Vec::new(),
            dns_flush_pending: true,
        });
        for change in changes {
            if record
                .changes
                .iter()
                .any(|owned| owned.resource == change.resource)
            {
                return Err(NetworkRecoveryError::InvalidRecord(
                    "同一网络资源已经由未完成事务持有".into(),
                ));
            }
            let original = self.backend.read(&change.resource).map_err(backend_error)?;
            record.changes.push(OwnedChange {
                resource: change.resource.clone(),
                original,
                applied: change.value.clone(),
            });
            self.store.save(&record)?;
            if let Err(error) = self
                .backend
                .write(&change.resource, change.value.as_deref())
            {
                return Err(backend_error(error));
            }
        }
        Ok(())
    }

    pub fn restore(&mut self) -> Result<NetworkRecoveryReport, NetworkRecoveryError> {
        let Some(mut record) = self.store.load()? else {
            return Ok(NetworkRecoveryReport {
                complete: true,
                restored: Vec::new(),
                preserved_external: Vec::new(),
                pending: Vec::new(),
                dns_flush_pending: false,
                application_cache_notice: false,
            });
        };
        let mut restored = Vec::new();
        let mut preserved_external = Vec::new();
        let mut pending = Vec::new();
        let mut remaining = Vec::new();
        for change in record.changes.iter().rev() {
            match self.backend.read(&change.resource) {
                Ok(current) if current == change.original => {
                    restored.push(change.resource.clone());
                }
                Ok(current) if current != change.applied => {
                    preserved_external.push(change.resource.clone());
                }
                Ok(_) => match self
                    .backend
                    .write(&change.resource, change.original.as_deref())
                {
                    Ok(()) => restored.push(change.resource.clone()),
                    Err(_) => {
                        pending.push(change.resource.clone());
                        remaining.push(change.clone());
                    }
                },
                Err(_) => {
                    pending.push(change.resource.clone());
                    remaining.push(change.clone());
                }
            }
        }
        remaining.reverse();
        record.changes = remaining;
        if record.changes.is_empty()
            && record.dns_flush_pending
            && self.backend.flush_dns_cache().is_ok()
        {
            record.dns_flush_pending = false;
        }
        let complete = record.changes.is_empty() && !record.dns_flush_pending;
        if complete {
            self.store.clear()?;
        } else {
            self.store.save(&record)?;
        }
        Ok(NetworkRecoveryReport {
            complete,
            restored,
            preserved_external,
            pending,
            dns_flush_pending: record.dns_flush_pending,
            application_cache_notice: true,
        })
    }

    pub fn into_backend(self) -> B {
        self.backend
    }
}

fn backend_error(error: impl fmt::Display) -> NetworkRecoveryError {
    NetworkRecoveryError::Backend(format!("网络设置操作失败: {error}"))
}

#[cfg(windows)]
fn replace_file(source: &std::path::Path, destination: &std::path::Path) -> io::Result<()> {
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
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &std::path::Path, destination: &std::path::Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use uuid::Uuid;

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
                return Err(io::Error::other("injected write failure"));
            }
            self.values
                .insert(resource.clone(), value.map(str::to_owned));
            Ok(())
        }

        fn flush_dns_cache(&mut self) -> Result<(), Self::Error> {
            if self.fail_flushes > 0 {
                self.fail_flushes -= 1;
                return Err(io::Error::other("injected flush failure"));
            }
            self.flushes += 1;
            Ok(())
        }
    }

    fn resource() -> NetworkResource {
        NetworkResource::DnsServers {
            interface_id: "fixture-interface".into(),
        }
    }

    fn manager(backend: Backend) -> (NetworkRecovery<Backend>, PathBuf) {
        let path = std::env::temp_dir().join(format!("network-recovery-{}.json", Uuid::new_v4()));
        let manager = NetworkRecovery::new(backend, RecoveryStore::new(&path));
        (manager, path)
    }

    #[test]
    fn restores_only_owned_values_and_preserves_external_changes() {
        let target = resource();
        let route = NetworkResource::Route {
            destination: "198.18.0.0/15".into(),
        };
        let mut backend = Backend::default();
        backend
            .values
            .insert(target.clone(), Some("system-dns".into()));
        backend.values.insert(route.clone(), None);
        let (mut manager, path) = manager(backend);
        manager
            .apply(&[
                NetworkChange {
                    resource: target.clone(),
                    value: Some("managed-dns".into()),
                },
                NetworkChange {
                    resource: route.clone(),
                    value: Some("owned-route".into()),
                },
            ])
            .unwrap();
        manager
            .backend
            .values
            .insert(target.clone(), Some("vpn-dns".into()));
        let report = manager.restore().unwrap();
        assert!(report.complete);
        assert_eq!(report.preserved_external, vec![target.clone()]);
        assert!(report.restored.contains(&route));
        assert_eq!(manager.backend.values[&target], Some("vpn-dns".into()));
        assert_eq!(manager.backend.values[&route], None);
        assert!(report.application_cache_notice);
        assert!(!path.exists());
    }

    #[test]
    fn failed_restore_and_dns_flush_are_retryable_after_restart() {
        let target = resource();
        let mut backend = Backend::default();
        backend.values.insert(target.clone(), Some("before".into()));
        let (mut manager, path) = manager(backend);
        manager
            .apply(&[NetworkChange {
                resource: target.clone(),
                value: Some("owned".into()),
            }])
            .unwrap();
        manager.backend.fail_writes = 1;
        let first = manager.restore().unwrap();
        assert!(!first.complete);
        assert_eq!(first.pending, vec![target.clone()]);
        assert!(path.exists());

        let mut backend = manager.into_backend();
        backend.fail_flushes = 1;
        let mut restarted = NetworkRecovery::new(backend, RecoveryStore::new(&path));
        let second = restarted.restore().unwrap();
        assert!(!second.complete);
        assert!(second.pending.is_empty());
        assert!(second.dns_flush_pending);
        let third = restarted.restore().unwrap();
        assert!(third.complete);
        assert_eq!(restarted.backend.values[&target], Some("before".into()));
        assert!(!path.exists());
    }

    #[test]
    fn prepared_record_recovers_when_a_crash_happens_after_the_write() {
        let target = resource();
        let (mut manager, path) = manager(Backend::default());
        let record = RecoveryRecord {
            schema_version: RECOVERY_SCHEMA_VERSION,
            changes: vec![OwnedChange {
                resource: target.clone(),
                original: None,
                applied: Some("owned".into()),
            }],
            dns_flush_pending: true,
        };
        manager.store.save(&record).unwrap();
        manager
            .backend
            .values
            .insert(target.clone(), Some("owned".into()));
        assert!(manager.restore().unwrap().complete);
        assert_eq!(manager.backend.values[&target], None);
        assert!(!path.exists());
    }
}
