#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    panic!("this validation example only runs on Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use socks_proxy::platform::windows::{SingleInstanceError, SingleInstanceGuard};
    use std::{env, process::Command};
    use windows_sys::Win32::System::Console::GetConsoleWindow;

    if env::args().any(|argument| argument == "--probe-second") {
        return match SingleInstanceGuard::acquire() {
            Err(SingleInstanceError::AlreadyRunning) => std::process::exit(23),
            Ok(_) => Err("second instance unexpectedly acquired the mutex".into()),
            Err(error) => Err(error.into()),
        };
    }

    let _instance = SingleInstanceGuard::acquire()?;
    if !unsafe { GetConsoleWindow() }.is_null() {
        return Err("GUI subsystem process has a console window".into());
    }
    let current = env::current_exe()?;
    let second = Command::new(current).arg("--probe-second").status()?;
    if second.code() != Some(23) {
        return Err(format!("unexpected second instance result: {second}").into());
    }
    println!("{{\"single_instance\":true,\"console_window\":false}}");
    Ok(())
}
