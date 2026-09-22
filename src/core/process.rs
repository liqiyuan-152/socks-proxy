use super::{
    BackendError, BackendStage, CoreBackend, CoreHandle, CoreProcessMonitor, RestrictedConfigFile,
};
use crate::logs::redact_sensitive;
use std::{
    fmt, io,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub enum ProcessCoreError {
    NotRunning,
    Io(io::Error),
}

type LineObserver = Arc<dyn Fn(&str) + Send + Sync>;

const CORE_STOP_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(windows)]
const TUN_CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum TunCleanupState {
    #[default]
    NotNeeded,
    Pending,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl TunCleanupState {
    fn mark_pending(&mut self) {
        *self = Self::Pending;
    }

    fn is_pending(self) -> bool {
        matches!(self, Self::Pending)
    }

    fn mark_complete(&mut self) {
        *self = Self::NotNeeded;
    }
}

impl fmt::Display for ProcessCoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning => formatter.write_str("内核未运行"),
            Self::Io(error) => write!(formatter, "内核进程操作失败: {error}"),
        }
    }
}

impl std::error::Error for ProcessCoreError {}

impl From<io::Error> for ProcessCoreError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub struct ProcessCoreBackend {
    core_path: PathBuf,
    config: Option<RestrictedConfigFile>,
    child: Option<Child>,
    line_receiver: Option<mpsc::Receiver<String>>,
    output: Arc<Mutex<String>>,
    line_observer: Option<LineObserver>,
    readiness_timeout: Duration,
    #[cfg(windows)]
    process_job: Option<ProcessJob>,
    #[cfg(windows)]
    tun_cleanup: TunCleanupState,
}

impl ProcessCoreBackend {
    pub fn new(core_path: impl Into<PathBuf>, config: RestrictedConfigFile) -> Self {
        Self {
            core_path: core_path.into(),
            config: Some(config),
            child: None,
            line_receiver: None,
            output: Arc::new(Mutex::new(String::new())),
            line_observer: None,
            readiness_timeout: default_readiness_timeout(),
            #[cfg(windows)]
            process_job: None,
            #[cfg(windows)]
            tun_cleanup: TunCleanupState::NotNeeded,
        }
    }

    pub fn set_line_observer(&mut self, observer: LineObserver) {
        self.line_observer = Some(observer);
    }

    pub fn core_path(&self) -> &Path {
        &self.core_path
    }

    pub fn captured_output(&self) -> String {
        self.output
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn try_wait(&mut self) -> Result<Option<i32>, ProcessCoreError> {
        let child = self.child.as_mut().ok_or(ProcessCoreError::NotRunning)?;
        Ok(child.try_wait()?.map(|status| status.code().unwrap_or(-1)))
    }

    pub fn stop(&mut self) -> Result<Option<i32>, ProcessCoreError> {
        let Some(mut child) = self.child.take() else {
            #[cfg(windows)]
            self.cleanup_owned_tun_adapter()?;
            return Ok(None);
        };
        #[cfg(windows)]
        let used_tun = self.captured_output().contains("inbound/tun[");
        #[cfg(windows)]
        if used_tun {
            self.tun_cleanup.mark_pending();
        }
        let exit_code = match child.try_wait()? {
            Some(status) => status.code(),
            None => {
                child.kill()?;
                let status = wait_for_child(&mut child, CORE_STOP_TIMEOUT, "停止内核进程")?;
                status.code()
            }
        };
        #[cfg(windows)]
        self.cleanup_owned_tun_adapter()?;
        self.line_receiver = None;
        self.config = None;
        #[cfg(windows)]
        {
            self.process_job = None;
        }
        Ok(exit_code)
    }

    #[cfg(test)]
    fn with_timeout(mut self, timeout: Duration) -> Self {
        self.readiness_timeout = timeout;
        self
    }

    #[cfg(windows)]
    fn cleanup_owned_tun_adapter(&mut self) -> Result<(), ProcessCoreError> {
        if !self.tun_cleanup.is_pending() {
            return Ok(());
        }
        remove_owned_tun_adapter()?;
        thread::sleep(Duration::from_millis(500));
        self.tun_cleanup.mark_complete();
        Ok(())
    }
}

fn default_readiness_timeout() -> Duration {
    if cfg!(windows) {
        Duration::from_secs(30)
    } else {
        Duration::from_secs(10)
    }
}

#[cfg(windows)]
fn remove_owned_tun_adapter() -> Result<(), ProcessCoreError> {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$names = @('socks-proxy-tun-v1', 'socks-proxy')
$root = 'HKLM:\SYSTEM\CurrentControlSet\Control\Network\{4D36E972-E325-11CE-BFC1-08002BE10318}'
$ids = @(
    Get-ChildItem $root | ForEach-Object {
        $connection = Get-ItemProperty (Join-Path $_.PSPath 'Connection') -ErrorAction SilentlyContinue
        if ($names -contains $connection.Name) { $connection.PnpInstanceID }
    } | Where-Object { $_ }
)
foreach ($id in $ids) {
    & pnputil.exe /remove-device $id | Out-Null
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
exit 0
"#;
    let mut command = hidden_command(Path::new("powershell.exe"));
    command.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]);
    let status =
        run_command_with_timeout(&mut command, TUN_CLEANUP_TIMEOUT, "清理应用 Wintun 设备")?;
    if status.success() {
        Ok(())
    } else {
        Err(ProcessCoreError::Io(io::Error::other(format!(
            "清理应用 Wintun 设备失败: {status}"
        ))))
    }
}

fn wait_for_child(
    child: &mut Child,
    timeout: Duration,
    operation: &str,
) -> io::Result<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{operation}超时"),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(any(windows, test))]
fn run_command_with_timeout(
    command: &mut Command,
    timeout: Duration,
    operation: &str,
) -> io::Result<std::process::ExitStatus> {
    let mut child = command.spawn()?;
    #[cfg(windows)]
    // Closing this job on any result terminates descendants spawned by the
    // cleanup shell, including pnputil.exe, instead of only its parent.
    let job = ProcessJob::assign(&child)?;
    let result = wait_for_child(&mut child, timeout, operation);
    #[cfg(windows)]
    drop(job);
    result
}

impl Drop for ProcessCoreBackend {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

impl CoreBackend for ProcessCoreBackend {
    fn version(&mut self) -> Result<String, BackendError> {
        let output = hidden_command(&self.core_path)
            .arg("version")
            .output()
            .map_err(|error| backend_error(BackendStage::Version, error))?;
        if !output.status.success() {
            return Err(BackendError {
                stage: BackendStage::Version,
                missing_binary: false,
                message: format!("内核版本命令失败: {}", output.status),
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_version(&stdout).ok_or_else(|| BackendError {
            stage: BackendStage::Version,
            missing_binary: false,
            message: "无法解析内核版本".into(),
        })
    }

    fn start(&mut self) -> Result<CoreHandle, BackendError> {
        if self.child.is_some() {
            return Err(BackendError {
                stage: BackendStage::Start,
                missing_binary: false,
                message: "内核已经运行".into(),
            });
        }
        let config = self.config.as_ref().ok_or_else(|| BackendError {
            stage: BackendStage::Start,
            missing_binary: false,
            message: "受限运行配置不可用".into(),
        })?;
        let mut child = hidden_command(&self.core_path)
            .arg("run")
            .arg("-c")
            .arg(config.path())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| backend_error(BackendStage::Start, error))?;
        #[cfg(windows)]
        let process_job = match ProcessJob::assign(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(backend_error(BackendStage::Start, error));
            }
        };
        let stderr = child.stderr.take().ok_or_else(|| BackendError {
            stage: BackendStage::Start,
            missing_binary: false,
            message: "无法捕获内核错误输出".into(),
        })?;
        let (sender, receiver) = mpsc::channel();
        let output = Arc::clone(&self.output);
        let observer = self.line_observer.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let line = redact_sensitive(&line);
                let mut captured = output.lock().unwrap_or_else(|error| error.into_inner());
                captured.push_str(&line);
                captured.push('\n');
                drop(captured);
                if let Some(observer) = &observer {
                    observer(&line);
                }
                let _ = sender.send(line);
            }
        });
        let id = child.id();
        self.child = Some(child);
        #[cfg(windows)]
        {
            self.process_job = Some(process_job);
        }
        self.line_receiver = Some(receiver);
        Ok(CoreHandle(id))
    }

    fn wait_ready(&mut self, handle: CoreHandle) -> Result<(), BackendError> {
        if self.child.as_ref().map(Child::id) != Some(handle.0) {
            return Err(BackendError {
                stage: BackendStage::Readiness,
                missing_binary: false,
                message: "内核句柄与运行进程不一致".into(),
            });
        }
        let deadline = Instant::now() + self.readiness_timeout;
        loop {
            if let Some(status) = self
                .child
                .as_mut()
                .expect("checked child")
                .try_wait()
                .map_err(|error| backend_error(BackendStage::Readiness, error))?
            {
                return Err(BackendError {
                    stage: BackendStage::Readiness,
                    missing_binary: false,
                    message: format!(
                        "内核就绪前退出 ({status}): {}",
                        self.captured_output().trim()
                    ),
                });
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(BackendError {
                    stage: BackendStage::Readiness,
                    missing_binary: false,
                    message: "等待内核就绪超时".into(),
                });
            }
            match self
                .line_receiver
                .as_ref()
                .expect("receiver exists while child runs")
                .recv_timeout(remaining.min(Duration::from_millis(200)))
            {
                Ok(line) if is_ready_line(&line) => return Ok(()),
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(BackendError {
                        stage: BackendStage::Readiness,
                        missing_binary: false,
                        message: format!("内核输出在就绪前关闭: {}", self.captured_output().trim()),
                    });
                }
            }
        }
    }
}

#[cfg(windows)]
struct ProcessJob(isize);

#[cfg(windows)]
impl ProcessJob {
    fn assign(child: &Child) -> io::Result<Self> {
        use std::{ffi::c_void, mem, os::windows::io::AsRawHandle, ptr};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast::<c_void>(),
                mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
            return Err(io::Error::last_os_error());
        }
        if unsafe { AssignProcessToJobObject(handle, child.as_raw_handle().cast()) } == 0 {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle as isize))
    }
}

#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(
                self.0 as windows_sys::Win32::Foundation::HANDLE,
            )
        };
    }
}

impl CoreProcessMonitor for ProcessCoreBackend {
    type Error = ProcessCoreError;

    fn try_wait(&mut self) -> Result<Option<i32>, Self::Error> {
        ProcessCoreBackend::try_wait(self)
    }

    fn captured_output(&self) -> String {
        ProcessCoreBackend::captured_output(self)
    }
}

fn parse_version(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        line.trim()
            .strip_prefix("sing-box version ")
            .map(str::to_owned)
    })
}

fn is_ready_line(line: &str) -> bool {
    line.contains("sing-box started") || line.contains("started (0.")
}

fn backend_error(stage: BackendStage, error: io::Error) -> BackendError {
    BackendError {
        stage,
        missing_binary: error.kind() == io::ErrorKind::NotFound,
        message: error.to_string(),
    }
}

fn hidden_command(program: &Path) -> Command {
    let command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = command;
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    {
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_the_pinned_version_line_shape() {
        assert_eq!(
            parse_version("sing-box version 1.14.1-socks-proxy.2\nEnvironment: go1.26"),
            Some("1.14.1-socks-proxy.2".into())
        );
        assert_eq!(parse_version("version unknown"), None);
    }

    #[test]
    fn recognizes_current_startup_log_shape() {
        assert!(is_ready_line("INFO sing-box started (0.12s)"));
        assert!(!is_ready_line("INFO initializing inbound/tun"));
    }

    #[test]
    fn missing_binary_is_classified_at_version_stage() {
        let directory = std::env::temp_dir().join(format!("missing-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("placeholder.json");
        std::fs::write(&path, b"{}").unwrap();
        let config = RestrictedConfigFile { path };
        let mut backend = ProcessCoreBackend::new(directory.join("missing-core"), config)
            .with_timeout(Duration::from_millis(10));
        let error = backend.version().unwrap_err();
        assert!(error.missing_binary);
        assert_eq!(error.stage, BackendStage::Version);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn wait_timeout_terminates_the_managed_child() {
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30"])
            .spawn()
            .expect("starts a child that exceeds the test timeout");
        let error = wait_for_child(&mut child, Duration::from_millis(1), "测试子进程").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "测试子进程超时");
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn tun_cleanup_stays_pending_until_the_cleanup_succeeds() {
        let mut cleanup = TunCleanupState::default();
        assert!(!cleanup.is_pending());
        cleanup.mark_pending();
        assert!(cleanup.is_pending());
        // A failed external cleanup leaves this state untouched for a retry.
        assert!(cleanup.is_pending());
        cleanup.mark_complete();
        assert!(!cleanup.is_pending());
    }

    #[cfg(unix)]
    #[test]
    fn external_cleanup_command_timeout_terminates_the_child() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30"]);
        let error =
            run_command_with_timeout(&mut command, Duration::from_millis(1), "测试外部清理命令")
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "测试外部清理命令超时");
    }
}
