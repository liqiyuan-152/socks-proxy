use crate::{
    core::{CoreCredentialSource, ProxyCredentials},
    domain::{CredentialRef, ProxyProfile},
    storage::CredentialVault,
};
use std::{fmt, fs, io, path::PathBuf, ptr};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, LocalFree},
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
    System::Threading::CreateMutexW,
};

const INSTANCE_MUTEX_NAME: &str = "Local\\socks-proxy-desktop-7f461940";

pub fn system_direct_dns_server() -> io::Result<crate::core::DirectDnsServer> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    if let Some(value) = std::env::var_os("SOCKS_PROXY_DIRECT_DNS") {
        let address = value.to_string_lossy().parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "SOCKS_PROXY_DIRECT_DNS 不是有效 IP",
            )
        })?;
        return Ok(crate::core::DirectDnsServer { address, port: 53 });
    }
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = "$up=(Get-NetIPInterface -ConnectionState Connected).InterfaceIndex; Get-DnsClientServerAddress | Where-Object {$up -contains $_.InterfaceIndex} | ForEach-Object {$_.ServerAddresses} | Select-Object -First 1";
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("无法读取当前网络的 DNS 服务器"));
    }
    let address = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .map_err(|_| io::Error::other("当前网络没有可用的 DNS 服务器"))?;
    Ok(crate::core::DirectDnsServer { address, port: 53 })
}

#[derive(Debug)]
pub enum SingleInstanceError {
    AlreadyRunning,
    Windows(io::Error),
}

impl fmt::Display for SingleInstanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => formatter.write_str("程序已经运行"),
            Self::Windows(error) => write!(formatter, "单实例检查失败: {error}"),
        }
    }
}

impl std::error::Error for SingleInstanceError {}

pub struct SingleInstanceGuard {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

impl SingleInstanceGuard {
    pub fn acquire() -> Result<Self, SingleInstanceError> {
        let name: Vec<u16> = INSTANCE_MUTEX_NAME.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(SingleInstanceError::Windows(io::Error::last_os_error()));
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            return Err(SingleInstanceError::AlreadyRunning);
        }
        Ok(Self { handle })
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

#[derive(Debug)]
pub enum CredentialError {
    Io(io::Error),
    Missing,
    Unavailable(io::Error),
    InvalidReference,
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "凭据存储读写失败: {error}"),
            Self::Missing => f.write_str("凭据不存在，请重新输入"),
            Self::Unavailable(error) => write!(f, "当前 Windows 账户无法解密凭据: {error}"),
            Self::InvalidReference => f.write_str("凭据引用无效"),
        }
    }
}

impl std::error::Error for CredentialError {}

impl From<io::Error> for CredentialError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub struct SecretBytes(Vec<u8>);

impl SecretBytes {
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for SecretBytes {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

pub struct DpapiCredentialVault {
    directory: PathBuf,
}

impl DpapiCredentialVault {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn read_version(&self, reference: &CredentialRef) -> Result<SecretBytes, CredentialError> {
        let path = self.path(reference)?;
        let protected = match fs::read(path) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(CredentialError::Missing);
            }
            Err(error) => return Err(CredentialError::Io(error)),
        };
        unprotect(&protected)
            .map(SecretBytes)
            .map_err(CredentialError::Unavailable)
    }

    fn path(&self, reference: &CredentialRef) -> Result<PathBuf, CredentialError> {
        CredentialRef::parse(reference.as_str()).map_err(|_| CredentialError::InvalidReference)?;
        Ok(self.directory.join(format!("{}.cred", reference.as_str())))
    }

    fn write_new(&self, reference: &CredentialRef, bytes: &[u8]) -> Result<(), CredentialError> {
        fs::create_dir_all(&self.directory)?;
        let target = self.path(reference)?;
        let temporary = self
            .directory
            .join(format!(".credential-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &target)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(CredentialError::Io)
    }
}

impl CredentialVault for DpapiCredentialVault {
    type Error = CredentialError;

    fn create_version(&mut self, secret: &[u8]) -> Result<CredentialRef, Self::Error> {
        let reference = CredentialRef::parse(&format!("credential-{}", Uuid::new_v4()))
            .map_err(|_| CredentialError::InvalidReference)?;
        let protected = protect(secret).map_err(CredentialError::Unavailable)?;
        self.write_new(&reference, &protected)?;
        Ok(reference)
    }

    fn delete_version(&mut self, reference: &CredentialRef) -> Result<(), Self::Error> {
        match fs::remove_file(self.path(reference)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(CredentialError::Io(error)),
        }
    }
}

impl CoreCredentialSource for DpapiCredentialVault {
    fn load(&self, profile: &ProxyProfile) -> Result<Option<ProxyCredentials>, String> {
        if !profile.auth_enabled {
            return Ok(None);
        }
        let reference = profile
            .credential_ref
            .as_ref()
            .ok_or_else(|| "当前代理需要补充认证凭据".to_owned())?;
        let secret = self
            .read_version(reference)
            .map_err(|error| error.to_string())?;
        let (username, password): (String, String) =
            serde_json::from_slice(secret.expose()).map_err(|_| "代理凭据内容无效".to_owned())?;
        Ok(Some(ProxyCredentials::new(username, password)))
    }
}

fn protect(secret: &[u8]) -> io::Result<Vec<u8>> {
    crypt(secret, true)
}

fn unprotect(ciphertext: &[u8]) -> io::Result<Vec<u8>> {
    crypt(ciphertext, false)
}

fn crypt(input: &[u8], protect: bool) -> io::Result<Vec<u8>> {
    let length = u32::try_from(input.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "凭据过长"))?;
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: length,
        pbData: input.as_ptr().cast_mut(),
    };
    let mut output_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let succeeded = unsafe {
        if protect {
            CryptProtectData(
                &input_blob,
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output_blob,
            )
        } else {
            CryptUnprotectData(
                &input_blob,
                ptr::null_mut(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output_blob,
            )
        }
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    let output = unsafe {
        let slice = std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize);
        let copied = slice.to_vec();
        ptr::write_bytes(output_blob.pbData, 0, output_blob.cbData as usize);
        LocalFree(output_blob.pbData.cast());
        copied
    };
    Ok(output)
}
