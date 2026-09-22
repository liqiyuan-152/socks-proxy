#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> eframe::Result {
    #[cfg(windows)]
    let _instance = {
        use socks_proxy::platform::windows::{SingleInstanceError, SingleInstanceGuard};
        match SingleInstanceGuard::acquire() {
            Ok(instance) => instance,
            Err(SingleInstanceError::AlreadyRunning) => return Ok(()),
            Err(error) => panic!("{error}"),
        }
    };

    let data_directory = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("SocksProxy");
    let store = socks_proxy::storage::ConfigStore::new(data_directory.join("config.json"));
    let config = store
        .load()
        .unwrap_or_else(|error| panic!("无法加载应用配置: {error}"));
    #[cfg(windows)]
    let credential_directory = data_directory.join("credentials");
    #[cfg(windows)]
    let vault = socks_proxy::platform::windows::DpapiCredentialVault::new(&credential_directory);
    #[cfg(not(windows))]
    let vault = socks_proxy::ui::MemoryCredentialVault::default();
    #[cfg(windows)]
    let runtime = {
        let core_path = std::env::var_os("SOCKS_PROXY_CORE_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .expect("无法定位应用程序")
                    .parent()
                    .expect("应用程序路径没有父目录")
                    .join("sing-box.exe")
            });
        let direct_dns = socks_proxy::platform::windows::system_direct_dns_server()
            .unwrap_or_else(|error| panic!("无法读取直连 DNS: {error}"));
        socks_proxy::core::ManagedCoreRuntime::new(
            core_path,
            data_directory.join("runtime"),
            data_directory.join("cache.db"),
            direct_dns,
            socks_proxy::platform::windows::DpapiCredentialVault::new(&credential_directory),
        )
    };
    #[cfg(not(windows))]
    let runtime = socks_proxy::ui::DirectOnlyRuntime;
    let mut controller =
        socks_proxy::ui::ManagedDesktopController::new(config, runtime, store, vault)
            .unwrap_or_else(|error| panic!("无法初始化应用控制器: {error}"));
    if std::env::args_os().any(|argument| argument == "--recover-direct") {
        socks_proxy::ui::SharedController::switch_mode(
            &mut controller,
            socks_proxy::routing::RoutingMode::Direct,
            true,
        )
        .unwrap_or_else(|error| panic!("无法恢复全局直连: {error}"));
        return Ok(());
    }
    let controller: std::sync::Arc<std::sync::Mutex<Box<dyn socks_proxy::ui::SharedController>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Box::new(controller)));

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Socks Proxy")
            .with_inner_size([1000.0, 680.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Socks Proxy",
        options,
        Box::new(move |context| {
            Ok(Box::new(socks_proxy::ui::DesktopApp::new(
                context, controller,
            )))
        }),
    )
}
