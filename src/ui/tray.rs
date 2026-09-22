use super::{Page, UiState};
use crate::routing::RoutingMode;
use muda::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Debug, PartialEq, Eq)]
pub enum TrayCommand {
    ToggleWindow,
    Mode(RoutingMode),
    Profile(String),
    Page(Page),
    Exit,
}

impl TrayCommand {
    pub fn requires_window_feedback(&self) -> bool {
        matches!(self, Self::Mode(_) | Self::Profile(_) | Self::Page(_))
    }
}

pub struct TrayManager {
    tray: TrayIcon,
    status_item: MenuItem,
    mode_items: Vec<(RoutingMode, CheckMenuItem)>,
    profile_items: Vec<(String, CheckMenuItem)>,
    profile_signature: Vec<(String, String)>,
    applied_mode: Option<RoutingMode>,
    active_profile_id: Option<String>,
    icon_state: Option<IconState>,
    tooltip: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IconState {
    Direct,
    Proxy,
    Error,
}

impl TrayManager {
    pub fn new(state: &UiState) -> Result<Self, String> {
        let (menu, status_item, mode_items, profile_items, profile_signature) = build_menu(state)?;
        let icon_state = icon_state(state);
        let tooltip = tooltip(state);
        let tray = TrayIconBuilder::new()
            .with_id("socks-proxy-main")
            .with_guid(0x7f461940_79c4_4a26_a1b2_b1a8ac3e11d2)
            .with_tooltip(&tooltip)
            .with_icon(status_icon(icon_state)?)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(true)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            tray,
            status_item,
            mode_items,
            profile_items,
            profile_signature,
            applied_mode: state.runtime.applied_mode,
            active_profile_id: active_profile_id(state),
            icon_state: Some(icon_state),
            tooltip,
        })
    }

    pub fn sync(&mut self, state: &UiState, mode_switching: bool) -> Result<(), String> {
        let signature = profile_signature(state);
        if signature != self.profile_signature {
            let (menu, status_item, mode_items, profile_items, profile_signature) =
                build_menu(state)?;
            self.tray.set_menu(Some(Box::new(menu)));
            self.status_item = status_item;
            self.mode_items = mode_items;
            self.profile_items = profile_items;
            self.profile_signature = profile_signature;
        }
        if self.applied_mode != state.runtime.applied_mode {
            for (mode, item) in &self.mode_items {
                item.set_checked(state.runtime.applied_mode == Some(*mode));
            }
            self.applied_mode = state.runtime.applied_mode;
        }
        let active_profile_id = active_profile_id(state);
        if self.active_profile_id != active_profile_id {
            for (id, item) in &self.profile_items {
                item.set_checked(active_profile_id.as_deref() == Some(id));
            }
            self.active_profile_id = active_profile_id;
        }
        let next_icon = icon_state(state);
        if self.icon_state != Some(next_icon) {
            // Explorer can report E_FAIL after accepting a replacement icon. Keep
            // runtime decoration best-effort so text/state synchronization continues.
            let _ = self.tray.set_icon(Some(status_icon(next_icon)?));
            self.icon_state = Some(next_icon);
        }
        let tooltip = if mode_switching {
            format!("正在应用代理模式 - {}", tooltip(state))
        } else {
            tooltip(state)
        };
        if self.tooltip != tooltip {
            // Explorer can report E_FAIL after accepting a tooltip update, just
            // as it does for replacement icons. Keep state synchronization going.
            let _ = self.tray.set_tooltip(Some(&tooltip));
            self.status_item.set_text(&tooltip);
            self.tooltip = tooltip;
        }
        Ok(())
    }

    pub fn poll(&self) -> Vec<TrayCommand> {
        let mut commands = Vec::new();
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                commands.push(TrayCommand::ToggleWindow);
            }
        }
        while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
            let id = event.id.0;
            let command = command_from_menu_id(&id);
            if let Some(command) = command {
                commands.push(command);
            }
        }
        commands
    }
}

fn command_from_menu_id(id: &str) -> Option<TrayCommand> {
    match id {
        "mode:direct" => Some(TrayCommand::Mode(RoutingMode::Direct)),
        "mode:rules" => Some(TrayCommand::Mode(RoutingMode::Rules)),
        "mode:global" => Some(TrayCommand::Mode(RoutingMode::GlobalProxy)),
        "page:status" => Some(TrayCommand::Page(Page::Status)),
        "page:proxies" => Some(TrayCommand::Page(Page::Proxies)),
        "page:rules" => Some(TrayCommand::Page(Page::Rules)),
        "page:logs" => Some(TrayCommand::Page(Page::Logs)),
        "page:settings" => Some(TrayCommand::Page(Page::Settings)),
        "exit" => Some(TrayCommand::Exit),
        _ => id
            .strip_prefix("profile:")
            .filter(|profile| !profile.is_empty())
            .map(|profile| TrayCommand::Profile(profile.to_owned())),
    }
}

type MenuParts = (
    Menu,
    MenuItem,
    Vec<(RoutingMode, CheckMenuItem)>,
    Vec<(String, CheckMenuItem)>,
    Vec<(String, String)>,
);

fn build_menu(state: &UiState) -> Result<MenuParts, String> {
    let menu = Menu::new();
    let status = MenuItem::with_id("status", tooltip(state), false, None);
    let modes = Submenu::with_id("modes", "代理模式", true);
    let mode_items = [
        (RoutingMode::Rules, "规则代理", "mode:rules"),
        (RoutingMode::GlobalProxy, "全局代理", "mode:global"),
        (RoutingMode::Direct, "全局直连", "mode:direct"),
    ]
    .into_iter()
    .map(|(mode, label, id)| {
        (
            mode,
            CheckMenuItem::with_id(
                id,
                label,
                true,
                state.runtime.applied_mode == Some(mode),
                None,
            ),
        )
    })
    .collect::<Vec<_>>();
    for (_, item) in &mode_items {
        modes.append(item).map_err(|error| error.to_string())?;
    }

    let profiles = Submenu::with_id("profiles", "切换代理", true);
    let profile_items = state
        .config
        .profiles
        .iter()
        .map(|profile| {
            let id = profile.id.as_str().to_owned();
            (
                id.clone(),
                CheckMenuItem::with_id(
                    format!("profile:{id}"),
                    &profile.name,
                    true,
                    state.config.profiles.active_id() == Some(&profile.id),
                    None,
                ),
            )
        })
        .collect::<Vec<_>>();
    for (_, item) in &profile_items {
        profiles.append(item).map_err(|error| error.to_string())?;
    }

    let separator_a = PredefinedMenuItem::separator();
    let separator_b = PredefinedMenuItem::separator();
    let exit = MenuItem::with_id("exit", "退出", true, None);
    let pages = [
        MenuItem::with_id("page:status", "状态", true, None),
        MenuItem::with_id("page:proxies", "代理管理...", true, None),
        MenuItem::with_id("page:rules", "规则管理...", true, None),
        MenuItem::with_id("page:logs", "查看连接日志...", true, None),
        MenuItem::with_id("page:settings", "设置...", true, None),
    ];
    menu.append(&status).map_err(|error| error.to_string())?;
    menu.append(&separator_a)
        .map_err(|error| error.to_string())?;
    menu.append(&modes).map_err(|error| error.to_string())?;
    menu.append(&profiles).map_err(|error| error.to_string())?;
    for page in &pages {
        menu.append(page).map_err(|error| error.to_string())?;
    }
    menu.append(&separator_b)
        .map_err(|error| error.to_string())?;
    menu.append(&exit).map_err(|error| error.to_string())?;

    Ok((
        menu,
        status,
        mode_items,
        profile_items,
        profile_signature(state),
    ))
}

fn profile_signature(state: &UiState) -> Vec<(String, String)> {
    state
        .config
        .profiles
        .iter()
        .map(|profile| (profile.id.as_str().to_owned(), profile.name.clone()))
        .collect()
}

fn active_profile_id(state: &UiState) -> Option<String> {
    state
        .config
        .profiles
        .active_id()
        .map(|id| id.as_str().to_owned())
}

fn icon_state(state: &UiState) -> IconState {
    if state.runtime.phase == crate::core::SwitchPhase::Error {
        IconState::Error
    } else if state.runtime.applied_mode == Some(RoutingMode::Direct) {
        IconState::Direct
    } else {
        IconState::Proxy
    }
}

fn tooltip(state: &UiState) -> String {
    let status = status_label(state.runtime.phase, state.runtime.applied_mode);
    match state.config.profiles.active() {
        Some(profile)
            if state.runtime.phase == crate::core::SwitchPhase::Error
                || state.runtime.applied_mode != Some(RoutingMode::Direct) =>
        {
            format!("Socks Proxy - {status} - {}", profile.name)
        }
        _ => format!("Socks Proxy - {status}"),
    }
}

fn status_label(
    phase: crate::core::SwitchPhase,
    applied_mode: Option<RoutingMode>,
) -> &'static str {
    if phase == crate::core::SwitchPhase::Error {
        return "运行异常";
    }
    match applied_mode {
        Some(RoutingMode::Direct) => "全局直连",
        Some(RoutingMode::Rules) => "规则代理",
        Some(RoutingMode::GlobalProxy) => "全局代理",
        None => "运行异常",
    }
}

fn status_icon(state: IconState) -> Result<Icon, String> {
    let color = match state {
        IconState::Direct => [70, 124, 89, 255],
        IconState::Proxy => [36, 99, 167, 255],
        IconState::Error => [181, 55, 55, 255],
    };
    let mut rgba = vec![0; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let dx = x as i32 - 16;
            let dy = y as i32 - 16;
            if dx * dx + dy * dy <= 13 * 13 {
                let offset = (y * 32 + x) * 4;
                rgba[offset..offset + 4].copy_from_slice(&color);
            }
        }
    }
    Icon::from_rgba(rgba, 32, 32).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{TrayCommand, command_from_menu_id, status_label};
    use crate::{core::SwitchPhase, routing::RoutingMode};

    #[test]
    fn error_phase_overrides_the_last_applied_mode_in_tray_status() {
        assert_eq!(
            status_label(SwitchPhase::Error, Some(RoutingMode::Rules)),
            "运行异常"
        );
        assert_eq!(
            status_label(SwitchPhase::Running, Some(RoutingMode::Rules)),
            "规则代理"
        );
    }

    #[test]
    fn every_enabled_menu_command_has_a_handler() {
        for id in [
            "mode:direct",
            "mode:rules",
            "mode:global",
            "page:status",
            "page:proxies",
            "page:rules",
            "page:logs",
            "page:settings",
            "exit",
            "profile:abc",
        ] {
            assert!(command_from_menu_id(id).is_some(), "{id}");
        }
        assert!(command_from_menu_id("profile:").is_none());
        assert!(command_from_menu_id("unknown").is_none());
        assert!(matches!(
            command_from_menu_id("exit"),
            Some(TrayCommand::Exit)
        ));
    }

    #[test]
    fn command_feedback_contract_matches_ui_actions() {
        let window_commands = [
            TrayCommand::Mode(RoutingMode::Rules),
            TrayCommand::Profile("proxy-a".into()),
            TrayCommand::Page(crate::ui::Page::Logs),
        ];
        for command in window_commands {
            assert!(command.requires_window_feedback());
        }
        assert!(!TrayCommand::ToggleWindow.requires_window_feedback());
        assert!(!TrayCommand::Exit.requires_window_feedback());
    }

    #[test]
    fn menu_ids_map_to_the_expected_actions() {
        assert_eq!(
            command_from_menu_id("mode:rules"),
            Some(TrayCommand::Mode(RoutingMode::Rules))
        );
        assert_eq!(
            command_from_menu_id("mode:global"),
            Some(TrayCommand::Mode(RoutingMode::GlobalProxy))
        );
        assert_eq!(
            command_from_menu_id("mode:direct"),
            Some(TrayCommand::Mode(RoutingMode::Direct))
        );
        assert_eq!(
            command_from_menu_id("page:logs"),
            Some(TrayCommand::Page(crate::ui::Page::Logs))
        );
        assert_eq!(
            command_from_menu_id("profile:proxy-a"),
            Some(TrayCommand::Profile("proxy-a".into()))
        );
    }
}
