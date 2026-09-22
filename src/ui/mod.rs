mod components;
mod controller;
mod pages;
mod theme;
#[cfg(windows)]
mod tray;
pub use controller::{
    BackupPreview, DirectOnlyRuntime, ManagedDesktopController, MemoryCredentialVault,
    ModeSwitchJob, ProxyDraft, RuleDraft, RuleTargetKind, SharedController, UiControlError,
    UiState,
};

use eframe::egui::{self, Align, Color32, FontId, Layout, RichText, Stroke, Vec2, ViewportCommand};
#[cfg(windows)]
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use crate::{
    domain::{ProfileId, ProxyProtocol},
    routing::RoutingMode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Status,
    Proxies,
    Rules,
    Logs,
    Settings,
}

impl Page {
    const ALL: [Self; 5] = [
        Self::Status,
        Self::Proxies,
        Self::Rules,
        Self::Logs,
        Self::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Status => "状态",
            Self::Proxies => "代理",
            Self::Rules => "分流规则",
            Self::Logs => "连接日志",
            Self::Settings => "设置",
        }
    }

    fn icon(self) -> egui::ImageSource<'static> {
        match self {
            Self::Status => egui::include_image!("../../assets/lucide/status.png"),
            Self::Proxies => egui::include_image!("../../assets/lucide/proxies.png"),
            Self::Rules => egui::include_image!("../../assets/lucide/rules.png"),
            Self::Logs => egui::include_image!("../../assets/lucide/logs.png"),
            Self::Settings => egui::include_image!("../../assets/lucide/settings.png"),
        }
    }

    #[cfg(test)]
    fn icon_resource_name(self) -> &'static str {
        match self {
            Self::Status => "status.png",
            Self::Proxies => "proxies.png",
            Self::Rules => "rules.png",
            Self::Logs => "logs.png",
            Self::Settings => "settings.png",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct NavigationLayout {
    compact: bool,
    width: f32,
}

fn navigation_layout(available_width: f32) -> NavigationLayout {
    let compact = available_width < theme::NAVIGATION_COMPACT_BREAKPOINT;
    NavigationLayout {
        compact,
        width: if compact {
            theme::NAVIGATION_COMPACT_WIDTH
        } else {
            theme::NAVIGATION_WIDE_WIDTH
        },
    }
}

pub struct DesktopApp {
    page: Page,
    controller: Arc<Mutex<Box<dyn SharedController>>>,
    proxy_draft: Option<ProxyDraft>,
    rule_draft: Option<RuleDraft>,
    #[cfg(windows)]
    pending_import: Option<(Vec<u8>, BackupPreview)>,
    pending: Option<PendingAction>,
    last_error: Option<UiControlError>,
    selected_log_connection: Option<u64>,
    selected_proxy: Option<ProfileId>,
    selected_rule: Option<String>,
    rule_filter: String,
    log_filter: String,
    log_outbound: Option<crate::logs::ConnectionOutbound>,
    log_success: Option<bool>,
    log_page_filter: crate::logs::ConnectionLogFilter,
    log_page_events: Vec<crate::logs::ConnectionEvent>,
    log_next_cursor: Option<crate::logs::ConnectionLogCursor>,
    log_page_loaded: bool,
    state_cache: UiState,
    mode_switch_job: ModeSwitchJob,
    #[cfg(windows)]
    exit_state: ExitState,
    #[cfg(windows)]
    window_visible: bool,
    #[cfg(windows)]
    exit_receiver: Option<Receiver<Result<(), UiControlError>>>,
    #[cfg(windows)]
    tray: Option<tray::TrayManager>,
}

#[derive(Clone)]
enum PendingAction {
    Proxy(ProfileId),
    Save(ProxyDraft),
    SaveRule(RuleDraft),
    DeleteRule(String),
    ToggleRule(String, bool),
}

impl DesktopApp {
    pub fn new(
        context: &eframe::CreationContext<'_>,
        controller: Arc<Mutex<Box<dyn SharedController>>>,
    ) -> Self {
        configure_fonts(&context.egui_ctx);
        egui_extras::install_image_loaders(&context.egui_ctx);
        theme::configure(&context.egui_ctx);
        let state_cache = controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .state();
        let app = Self {
            page: Page::Status,
            controller,
            proxy_draft: None,
            rule_draft: None,
            #[cfg(windows)]
            pending_import: None,
            pending: None,
            last_error: None,
            selected_log_connection: None,
            selected_proxy: None,
            selected_rule: None,
            rule_filter: String::new(),
            log_filter: String::new(),
            log_outbound: None,
            log_success: None,
            log_page_filter: crate::logs::ConnectionLogFilter::default(),
            log_page_events: Vec::new(),
            log_next_cursor: None,
            log_page_loaded: false,
            state_cache: state_cache.clone(),
            mode_switch_job: ModeSwitchJob::new(state_cache.clone()),
            #[cfg(windows)]
            exit_state: ExitState::Idle,
            #[cfg(windows)]
            window_visible: true,
            #[cfg(windows)]
            exit_receiver: None,
            #[cfg(windows)]
            tray: None,
        };
        #[cfg(windows)]
        {
            let mut app = app;
            match tray::TrayManager::new(&app.state()) {
                Ok(tray) => app.tray = Some(tray),
                Err(error) if std::env::var_os("SOCKS_PROXY_TRAY_REQUIRED").is_some() => {
                    panic!("无法创建托盘: {error}")
                }
                Err(error) => app.last_error = Some(UiControlError::Operation(error)),
            }
            app
        }
        #[cfg(not(windows))]
        {
            app
        }
    }

    pub fn current_page(&self) -> Page {
        self.page
    }

    fn navigation(&mut self, root: &mut egui::Ui) {
        let navigation = navigation_layout(root.available_width());
        egui::Panel::left("navigation")
            .exact_size(navigation.width)
            .resizable(false)
            .frame(
                egui::Frame::side_top_panel(root.style())
                    .fill(theme::CANVAS)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .inner_margin(egui::Margin::symmetric(10, 14)),
            )
            .show(root, |ui| {
                let compact = navigation.compact;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("●").size(20.0).color(theme::BLUE));
                    if !compact {
                        ui.label(
                            RichText::new("Socks Proxy")
                                .font(FontId::proportional(18.0))
                                .strong()
                                .color(theme::TEXT),
                        );
                    }
                });
                ui.add_space(18.0);
                for page in Page::ALL {
                    let selected = self.page == page;
                    let icon = egui::Image::new(page.icon())
                        .fit_to_exact_size(Vec2::splat(theme::ICON_SIZE));
                    let response = if compact {
                        components::icon_button(
                            ui,
                            icon,
                            page.label(),
                            Vec2::new(ui.available_width(), theme::NAVIGATION_ROW_HEIGHT),
                            true,
                            selected,
                        )
                    } else {
                        let button =
                            egui::Button::image_and_text(icon, page.label()).selected(selected);
                        ui.add_sized([ui.available_width(), theme::NAVIGATION_ROW_HEIGHT], button)
                            .on_hover_text(page.label())
                    };
                    if response.clicked() {
                        self.page = page;
                    }
                    ui.add_space(4.0);
                }
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    if compact {
                        ui.label(RichText::new("v1.3").small().color(theme::MUTED));
                    } else {
                        ui.label(
                            RichText::new("Windows 10/11 x64")
                                .small()
                                .color(theme::MUTED),
                        );
                    }
                });
            });
    }

    fn state(&self) -> UiState {
        if self.mode_switch_job.is_running() {
            return self.mode_switch_job.snapshot().clone();
        }
        let mut controller = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        controller.state()
    }

    fn status_bar(&self, root: &mut egui::Ui, state: &UiState) {
        egui::Panel::bottom("status_bar")
            .exact_size(34.0)
            .frame(
                egui::Frame::side_top_panel(root.style())
                    .fill(Color32::from_rgb(247, 248, 250))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(220, 223, 228)))
                    .inner_margin(egui::Margin::symmetric(16, 8)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let (color, text) = match state.runtime.applied_mode {
                        Some(RoutingMode::Direct) => (Color32::from_rgb(70, 124, 89), "全局直连"),
                        Some(RoutingMode::Rules) => (Color32::from_rgb(36, 99, 167), "规则代理"),
                        Some(RoutingMode::GlobalProxy) => {
                            (Color32::from_rgb(36, 99, 167), "全局代理")
                        }
                        None => (Color32::from_rgb(181, 55, 55), "异常"),
                    };
                    components::state_badge(ui, color, text);
                    ui.separator();
                    let proxy_name = state
                        .config
                        .profiles
                        .active()
                        .map(|profile| profile.name.as_str())
                        .unwrap_or("未选择代理");
                    ui.label(RichText::new(format!("当前代理: {proxy_name}")).color(theme::MUTED));
                    ui.separator();
                    ui.label(
                        RichText::new(if state.runtime.traffic_may_be_direct {
                            "代理接管失效，流量可能直连"
                        } else {
                            "网络状态"
                        })
                        .color(theme::MUTED),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(global_phase_label(state.runtime.phase)).weak());
                    });
                });
            });
    }

    fn status_page(&mut self, ui: &mut egui::Ui, state: &UiState) {
        components::page_header(ui, "状态", "当前网络模式和实际运行状态", |_| {});
        let applying = is_applying(state) || self.mode_switch_job.is_running();
        ui.label(RichText::new("代理模式").strong().color(theme::TEXT));
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            for (mode, label) in [
                (RoutingMode::Direct, "全局直连"),
                (RoutingMode::Rules, "规则代理"),
                (RoutingMode::GlobalProxy, "全局代理"),
            ] {
                if ui
                    .add_enabled_ui(!applying, |ui| {
                        ui.add_sized(
                            [142.0, 38.0],
                            egui::Button::selectable(
                                state.runtime.applied_mode == Some(mode),
                                label,
                            ),
                        )
                    })
                    .inner
                    .clicked()
                    && state.runtime.applied_mode != Some(mode)
                {
                    self.request_mode(mode, false);
                }
            }
        });
        self.error_banner(ui);
        ui.add_space(16.0);
        ui.columns(2, |columns| {
            components::section(&mut columns[0], "当前代理信息", |ui| {
                let active = state.config.profiles.active();
                detail_grid(
                    ui,
                    [
                        (
                            "代理名称",
                            active.map_or("未选择".into(), |profile| profile.name.clone()),
                        ),
                        (
                            "协议",
                            active.map_or("-".into(), |profile| {
                                protocol_label(profile.protocol).into()
                            }),
                        ),
                        (
                            "服务器",
                            active.map_or("-".into(), |profile| {
                                format!("{}:{}", host_label(&profile.host), profile.port)
                            }),
                        ),
                        (
                            "认证状态",
                            active.map_or("-".into(), |profile| {
                                if profile.auth_enabled {
                                    "已启用".into()
                                } else {
                                    "未启用".into()
                                }
                            }),
                        ),
                    ],
                );
                ui.add_space(6.0);
                ui.add_enabled_ui(!applying, |ui| {
                    egui::ComboBox::from_id_salt("active_proxy")
                        .width(ui.available_width())
                        .selected_text("切换当前代理")
                        .show_ui(ui, |ui| {
                            for profile in state.config.profiles.iter() {
                                if ui
                                    .selectable_label(
                                        state.config.profiles.active_id() == Some(&profile.id),
                                        &profile.name,
                                    )
                                    .clicked()
                                    && state.config.profiles.active_id() != Some(&profile.id)
                                {
                                    self.request_proxy(profile.id.clone(), false);
                                }
                            }
                        });
                });
            });
            components::section(&mut columns[1], "内核状态", |ui| {
                detail_grid(
                    ui,
                    [
                        (
                            "当前模式",
                            state.runtime.applied_mode.map_or("异常", mode_label).into(),
                        ),
                        ("运行状态", phase_label(state.runtime.phase).into()),
                        (
                            "DNS 状态",
                            if state.runtime.applied_mode == Some(RoutingMode::Direct) {
                                "系统 DNS".into()
                            } else {
                                "受管理".into()
                            },
                        ),
                        (
                            "配置修订",
                            format!("修订 {}", state.runtime.applied_revision),
                        ),
                    ],
                );
            });
        });
        ui.add_space(16.0);
        let total_connections = state.connection_events.len();
        let successful_connections = state
            .connection_events
            .iter()
            .filter(|event| matches!(event.result, crate::logs::ConnectionResult::Success))
            .count();
        let failed_connections = total_connections.saturating_sub(successful_connections);
        components::section(ui, "连接统计（今日）", |ui| {
            ui.columns(4, |columns| {
                for (column, (label, value, color)) in columns.iter_mut().zip([
                    ("今日连接数", total_connections.to_string(), theme::TEXT),
                    ("成功数", successful_connections.to_string(), theme::GREEN),
                    ("失败数", failed_connections.to_string(), theme::RED),
                    (
                        "最近命中规则",
                        state
                            .connection_events
                            .first()
                            .and_then(|event| match &event.rule {
                                crate::logs::RuleAttribution::Known(name) => Some(name.as_str()),
                                crate::logs::RuleAttribution::Unknown => None,
                            })
                            .unwrap_or("暂无")
                            .to_owned(),
                        theme::BLUE,
                    ),
                ]) {
                    column.label(RichText::new(label).small().color(theme::MUTED));
                    column.label(RichText::new(value).strong().color(color));
                }
            });
        });
        if let Some(error) = state.runtime.failure.as_deref() {
            ui.add_space(12.0);
            components::error_banner(ui, error);
        }
        ui.add_space(16.0);
        components::section(ui, "最近连接记录", |ui| {
            if state.connection_events.is_empty() {
                ui.label(RichText::new("暂无连接记录").color(theme::MUTED));
                return;
            }
            table_header(ui, &["时间", "目标", "出站", "结果"]);
            for event in state.connection_events.iter().rev().take(3) {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [86.0, 26.0],
                        egui::Label::new(event_time(event.observed_at_unix_ms)),
                    );
                    ui.add_sized(
                        [220.0, 26.0],
                        egui::Label::new(format!("{}:{}", event.target, event.port)).truncate(),
                    );
                    ui.add_sized(
                        [72.0, 26.0],
                        egui::Label::new(outbound_label(event.outbound)),
                    );
                    match event.result {
                        crate::logs::ConnectionResult::Success => {
                            components::state_badge(ui, theme::GREEN, "成功")
                        }
                        crate::logs::ConnectionResult::Failure(_) => {
                            components::state_badge(ui, theme::RED, "失败")
                        }
                    }
                });
                ui.separator();
            }
        });
    }

    fn proxies_page(&mut self, ui: &mut egui::Ui, state: &UiState) {
        components::page_header(ui, "代理", "管理 SOCKS5 和 HTTP 代理配置", |ui| {
            if ui.button("+ 新建代理").clicked() {
                self.last_error = None;
                self.proxy_draft = Some(ProxyDraft::default());
            }
        });
        self.error_banner(ui);
        ui.columns(2, |columns| {
            components::section(&mut columns[0], "代理列表", |ui| {
                egui::ScrollArea::vertical()
                    .max_height(440.0)
                    .show(ui, |ui| {
                        for profile in state.config.profiles.iter() {
                            let selected = self.selected_proxy.as_ref() == Some(&profile.id)
                                || (self.selected_proxy.is_none()
                                    && state.config.profiles.active_id() == Some(&profile.id));
                            let label = format!(
                                "{}\n{}  {}:{}",
                                profile.name,
                                protocol_label(profile.protocol),
                                host_label(&profile.host),
                                profile.port
                            );
                            if ui
                                .add_sized(
                                    [ui.available_width(), 48.0],
                                    egui::Button::selectable(selected, label),
                                )
                                .clicked()
                            {
                                self.selected_proxy = Some(profile.id.clone());
                            }
                            ui.add_space(4.0);
                        }
                    });
            });
            components::section(&mut columns[1], "代理详情", |ui| {
                let selected = self
                    .selected_proxy
                    .as_ref()
                    .and_then(|id| state.config.profiles.get(id))
                    .or_else(|| state.config.profiles.active());
                if let Some(profile) = selected {
                    detail_grid(
                        ui,
                        [
                            ("名称", profile.name.clone()),
                            ("协议", protocol_label(profile.protocol).into()),
                            (
                                "服务器",
                                format!("{}:{}", host_label(&profile.host), profile.port),
                            ),
                            (
                                "认证",
                                if profile.auth_enabled {
                                    "已配置".into()
                                } else {
                                    "未启用".into()
                                },
                            ),
                        ],
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("编辑").clicked() {
                            self.last_error = None;
                            self.proxy_draft = Some(ProxyDraft::from_profile(profile));
                        }
                        if ui
                            .add_enabled(
                                state.config.profiles.active_id() != Some(&profile.id),
                                egui::Button::new("设为当前代理"),
                            )
                            .clicked()
                        {
                            self.request_proxy(profile.id.clone(), false);
                        }
                        if ui.small_button("删除").clicked() {
                            self.delete_proxy(&profile.id);
                        }
                    });
                } else {
                    ui.label(RichText::new("从左侧选择一个代理查看详情。 ").color(theme::MUTED));
                }
            });
        });
        if state.config.profiles.iter().next().is_none() {
            components::empty_state(ui, "暂无代理，创建一个代理后即可启用规则代理或全局代理。");
        }
    }

    fn rules_page(&mut self, ui: &mut egui::Ui, state: &UiState) {
        components::page_header(
            ui,
            "分流规则",
            "决定哪些目标通过当前代理连接",
            |ui| {
                if ui.button("+ 新建规则").clicked() {
                    self.last_error = None;
                    self.rule_draft = Some(RuleDraft::default());
                }
            },
        );
        self.error_banner(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new("筛选").color(theme::MUTED));
            ui.add_sized(
                [260.0, 30.0],
                egui::TextEdit::singleline(&mut self.rule_filter).hint_text("名称、目标或备注"),
            );
            if !self.rule_filter.is_empty() && ui.small_button("清除").clicked() {
                self.rule_filter.clear();
            }
        });
        ui.add_space(8.0);
        let rule_filter = self.rule_filter.clone();
        ui.columns(2, |columns| {
            components::section(&mut columns[0], "规则列表", |ui| {
                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .show(ui, |ui| {
                        for rule in state
                            .config
                            .rules
                            .iter()
                            .filter(|rule| rule_matches(rule, &rule_filter))
                        {
                            let selected = self.selected_rule.as_deref() == Some(rule.id.as_str());
                            let label = format!(
                                "{}\n{}  ·  {}",
                                rule.name,
                                rule_target_label(&rule.target),
                                rule_ports_label(&rule.ports)
                            );
                            ui.horizontal(|ui| {
                                let mut enabled = rule.enabled;
                                if ui
                                    .checkbox(&mut enabled, "")
                                    .on_hover_text("启用或停用规则")
                                    .changed()
                                {
                                    self.toggle_rule(rule.id.clone(), enabled, false);
                                }
                                if ui
                                    .add_sized(
                                        [ui.available_width(), 46.0],
                                        egui::Button::selectable(selected, label),
                                    )
                                    .clicked()
                                {
                                    self.selected_rule = Some(rule.id.clone());
                                }
                            });
                            ui.add_space(4.0);
                        }
                    });
            });
            components::section(&mut columns[1], "规则详情", |ui| {
                let selected = self
                    .selected_rule
                    .as_deref()
                    .and_then(|id| state.config.rules.iter().find(|rule| rule.id == id))
                    .or_else(|| {
                        state
                            .config
                            .rules
                            .iter()
                            .find(|rule| rule_matches(rule, &rule_filter))
                    });
                if let Some(rule) = selected {
                    detail_grid(
                        ui,
                        [
                            ("名称", rule.name.clone()),
                            ("目标", rule_target_label(&rule.target)),
                            ("端口", rule_ports_label(&rule.ports)),
                            (
                                "状态",
                                if rule.enabled {
                                    "已启用".into()
                                } else {
                                    "已停用".into()
                                },
                            ),
                            (
                                "备注",
                                if rule.note.is_empty() {
                                    "-".into()
                                } else {
                                    rule.note.clone()
                                },
                            ),
                        ],
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("编辑").clicked() {
                            self.last_error = None;
                            self.rule_draft = Some(RuleDraft::from_rule(rule));
                        }
                        if ui.small_button("删除").clicked() {
                            self.delete_rule(rule.id.clone(), false);
                        }
                    });
                } else {
                    ui.label(RichText::new("从左侧选择一条规则查看详情。 ").color(theme::MUTED));
                }
            });
        });
        if state.config.rules.is_empty() {
            components::empty_state(ui, "暂无规则，创建规则后可按目标和端口进行分流。");
        } else if !self.rule_filter.is_empty()
            && !state
                .config
                .rules
                .iter()
                .any(|rule| rule_matches(rule, &self.rule_filter))
        {
            components::empty_state(ui, "没有匹配的规则。");
        }
    }

    fn logs_page(&mut self, ui: &mut egui::Ui, state: &UiState) {
        components::page_header(
            ui,
            "连接日志",
            "查看已经脱敏的近期连接结果",
            |ui| {
                if ui
                    .add_enabled(!self.log_page_events.is_empty(), egui::Button::new("清空"))
                    .clicked()
                {
                    let result = self
                        .controller
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .clear_logs();
                    match result {
                        Ok(()) => {
                            clear_log_selection(&mut self.selected_log_connection);
                            self.log_page_events.clear();
                            self.log_next_cursor = None;
                            self.log_page_loaded = true;
                        }
                        Err(error) => {
                            self.last_error = Some(error);
                        }
                    }
                }
            },
        );
        self.error_banner(ui);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("筛选").color(theme::MUTED));
            ui.add_sized(
                [220.0, 30.0],
                egui::TextEdit::singleline(&mut self.log_filter).hint_text("目标或规则"),
            );
            egui::ComboBox::from_id_salt("log_outbound_filter")
                .width(92.0)
                .selected_text(self.log_outbound.map_or("全部出站", outbound_label))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.log_outbound, None, "全部出站");
                    for outbound in [
                        crate::logs::ConnectionOutbound::Direct,
                        crate::logs::ConnectionOutbound::Proxy,
                        crate::logs::ConnectionOutbound::Unknown,
                    ] {
                        ui.selectable_value(
                            &mut self.log_outbound,
                            Some(outbound),
                            outbound_label(outbound),
                        );
                    }
                });
            egui::ComboBox::from_id_salt("log_result_filter")
                .width(88.0)
                .selected_text(match self.log_success {
                    Some(true) => "成功",
                    Some(false) => "失败",
                    None => "全部结果",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.log_success, None, "全部结果");
                    ui.selectable_value(&mut self.log_success, Some(true), "成功");
                    ui.selectable_value(&mut self.log_success, Some(false), "失败");
                });
            if (!self.log_filter.is_empty()
                || self.log_outbound.is_some()
                || self.log_success.is_some())
                && ui.small_button("清除").clicked()
            {
                self.log_filter.clear();
                self.log_outbound = None;
                self.log_success = None;
            }
        });
        ui.add_space(8.0);
        let filter = self.current_log_filter();
        if (!self.log_page_loaded || filter != self.log_page_filter)
            && !self.mode_switch_job.is_running()
        {
            self.load_log_page(None);
        }
        if has_unread_log_events(&state.connection_events, &self.log_page_events) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("有新的连接日志").color(theme::MUTED));
                if ui.button("刷新").clicked() {
                    clear_log_selection(&mut self.selected_log_connection);
                    self.load_log_page(None);
                }
            });
            ui.add_space(6.0);
        }
        if self.log_page_events.is_empty() {
            components::empty_state(ui, "暂无连接记录。连接发生后会在这里显示脱敏摘要。");
            return;
        }
        components::section(ui, "连接记录", |ui| {
            table_header(ui, &["时间", "目标", "出站", "规则", "结果", ""]);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_rows(ui, 31.0, self.log_page_events.len(), |ui, row_range| {
                    for row in row_range {
                        let event = &self.log_page_events[row];
                        ui.horizontal(|ui| {
                            for value in [
                                event_time(event.observed_at_unix_ms),
                                format!("{}:{}", event.target, event.port),
                                match event.outbound {
                                    crate::logs::ConnectionOutbound::Direct => "直连".into(),
                                    crate::logs::ConnectionOutbound::Proxy => "代理".into(),
                                    crate::logs::ConnectionOutbound::Unknown => "未知".into(),
                                },
                                match &event.rule {
                                    crate::logs::RuleAttribution::Known(rule) => rule.clone(),
                                    crate::logs::RuleAttribution::Unknown => "未知".into(),
                                },
                                match &event.result {
                                    crate::logs::ConnectionResult::Success => "成功".into(),
                                    crate::logs::ConnectionResult::Failure(detail) => {
                                        format!("失败: {detail}")
                                    }
                                },
                            ] {
                                ui.add_sized(
                                    [132.0, 28.0],
                                    egui::Label::new(value)
                                        .truncate()
                                        .sense(egui::Sense::hover()),
                                );
                            }
                            let selected =
                                self.selected_log_connection == Some(event.connection_id);
                            if ui
                                .add_sized(
                                    [56.0, 28.0],
                                    egui::Button::selectable(
                                        selected,
                                        if selected { "收起" } else { "详情" },
                                    ),
                                )
                                .clicked()
                            {
                                self.selected_log_connection =
                                    (!selected).then_some(event.connection_id);
                            }
                        });
                    }
                });
            if let Some(cursor) = self.log_next_cursor
                && ui.button("加载更多").clicked()
            {
                self.load_log_page(Some(cursor));
            }
            if let Some(event) = self
                .log_page_events
                .iter()
                .find(|event| self.selected_log_connection == Some(event.connection_id))
            {
                ui.add_space(8.0);
                egui::Frame::group(ui.style())
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| log_detail(ui, event));
            }
        });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui, state: &UiState) {
        components::page_header(ui, "设置", "启动、备份与网络恢复", |_| {});
        components::section(ui, "启动", |ui| {
            let mut startup_enabled = state.config.preferences.start_with_windows;
            ui.add_enabled(false, egui::Checkbox::new(&mut startup_enabled, "开机启动"));
            ui.label(
                RichText::new("首版默认关闭，当前仅展示已保存偏好")
                    .small()
                    .color(theme::MUTED),
            );
        });
        ui.add_space(14.0);
        components::section(ui, "备份", |ui| {
            ui.label(
                RichText::new("导出文件不包含密码；导入前会先显示替换范围。 ").color(theme::MUTED),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                #[cfg(windows)]
                {
                    if ui.button("导入配置...").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Socks Proxy 备份", &["json"])
                            .pick_file()
                    {
                        match std::fs::read(path) {
                            Ok(bytes) => {
                                let preview = self
                                    .controller
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner())
                                    .preview_backup(&bytes);
                                match preview {
                                    Ok(preview) => {
                                        self.pending_import = Some((bytes, preview));
                                        self.last_error = None;
                                    }
                                    Err(error) => self.last_error = Some(error),
                                }
                            }
                            Err(error) => {
                                self.last_error = Some(UiControlError::Operation(format!(
                                    "读取备份失败: {error}"
                                )))
                            }
                        }
                    }
                    if ui.button("导出配置...").clicked() {
                        let backup = self
                            .controller
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .export_backup();
                        match backup {
                            Ok(bytes) => {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("Socks Proxy 备份", &["json"])
                                    .set_file_name("socks-proxy-backup.json")
                                    .save_file()
                                    && let Err(error) = std::fs::write(path, bytes)
                                {
                                    self.last_error = Some(UiControlError::Operation(format!(
                                        "写入备份失败: {error}"
                                    )));
                                }
                            }
                            Err(error) => self.last_error = Some(error),
                        }
                    }
                }
                #[cfg(not(windows))]
                {
                    ui.add_enabled(false, egui::Button::new("导入配置..."));
                    ui.add_enabled(false, egui::Button::new("导出配置..."));
                }
            });
        });
        self.error_banner(ui);
        ui.add_space(14.0);
        components::section(ui, "网络恢复", |ui| {
            components::state_badge(ui, theme::GREEN, "没有待恢复的网络修改");
        });
    }

    #[cfg(windows)]
    fn import_preview_dialog(&mut self, context: &egui::Context, state: &UiState) {
        let Some((_, preview)) = self.pending_import.as_ref() else {
            return;
        };
        let preview = *preview;
        let direct = pages::settings::import_allowed(state);
        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("确认导入备份")
            .collapsible(false)
            .resizable(false)
            .min_width(440.0)
            .open(&mut open)
            .show(context, |ui| {
                ui.label(RichText::new("导入预览").size(18.0).strong());
                ui.label(
                    RichText::new("导入将整体替换当前代理、规则和启动偏好。 ").color(theme::MUTED),
                );
                ui.add_space(12.0);
                components::section(ui, "将要导入", |ui| {
                    detail_grid(
                        ui,
                        [
                            ("代理", preview.profiles.to_string()),
                            ("规则", preview.rules.to_string()),
                            ("待补充认证", preview.missing_credentials.to_string()),
                        ],
                    );
                });
                if preview.missing_credentials > 0 {
                    ui.add_space(8.0);
                    components::operation_banner(
                        ui,
                        theme::BLUE,
                        "认证信息需要补充",
                        "备份不包含密码；导入后需重新填写认证信息。",
                    );
                }
                if !direct {
                    ui.add_space(8.0);
                    components::operation_banner(
                        ui,
                        theme::RED,
                        "当前不能导入",
                        "请先切换到全局直连并完成网络恢复。",
                    );
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    confirm = ui
                        .add_enabled(direct, egui::Button::new("确认导入"))
                        .clicked();
                    cancel = ui.button("取消").clicked();
                });
            });
        if confirm {
            let bytes = self.pending_import.take().map(|value| value.0);
            if let Some(bytes) = bytes {
                let result = self
                    .controller
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .import_backup(&bytes);
                match result {
                    Ok(()) => self.last_error = None,
                    Err(error) => {
                        self.last_error = Some(error);
                    }
                }
            }
        } else if cancel || !open {
            self.pending_import = None;
        }
    }

    fn request_mode(&mut self, mode: RoutingMode, _confirmed: bool) {
        if self.mode_switch_job.is_running() {
            self.last_error = Some(mode_switch_conflict_error());
            return;
        }
        self.pending = None;
        self.last_error = None;
        if let Err(error) = self
            .mode_switch_job
            .start(Arc::clone(&self.controller), mode)
        {
            self.last_error = Some(error);
        }
    }

    fn reject_conflicting_operation(&mut self) -> bool {
        if !self.mode_switch_job.is_running() {
            return false;
        }
        self.last_error = Some(mode_switch_conflict_error());
        true
    }

    fn poll_mode(&mut self) {
        if let Some(result) = self.mode_switch_job.poll() {
            match result {
                Ok(()) => self.last_error = None,
                Err(error) => self.last_error = Some(error),
            }
            self.state_cache = self.state();
            self.mode_switch_job
                .update_snapshot(self.state_cache.clone());
        }
    }

    fn request_proxy(&mut self, id: ProfileId, confirmed: bool) {
        if self.reject_conflicting_operation() {
            return;
        }
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .select_proxy(&id, confirmed);
        self.handle_action(result, PendingAction::Proxy(id));
    }

    fn delete_proxy(&mut self, id: &ProfileId) {
        if self.reject_conflicting_operation() {
            return;
        }
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .delete_proxy(id);
        match result {
            Ok(()) => self.last_error = None,
            Err(error) => {
                self.last_error = Some(error);
            }
        }
    }

    fn handle_action(&mut self, result: Result<(), UiControlError>, pending: PendingAction) {
        match result {
            Ok(()) => {
                self.pending = None;
                self.last_error = None;
            }
            Err(UiControlError::ConfirmationRequired(message)) => {
                self.pending = Some(pending);
                self.last_error = Some(UiControlError::ConfirmationRequired(message));
            }
            Err(error) => {
                self.pending = None;
                self.last_error = Some(error);
            }
        }
    }

    fn error_banner(&self, ui: &mut egui::Ui) {
        if let Some(error) = &self.last_error {
            ui.add_space(8.0);
            components::error_banner(ui, &error.to_string());
        }
    }

    fn current_log_filter(&self) -> crate::logs::ConnectionLogFilter {
        crate::logs::ConnectionLogFilter {
            query: self.log_filter.clone(),
            outbound: self.log_outbound,
            success: self.log_success,
        }
    }

    fn load_log_page(&mut self, cursor: Option<crate::logs::ConnectionLogCursor>) {
        if self.mode_switch_job.is_running() {
            return;
        }
        let filter = self.current_log_filter();
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .connection_log_page(cursor, &filter);
        match result {
            Ok(page) => {
                if cursor.is_none() {
                    self.log_page_events = page.events;
                    self.log_page_filter = filter;
                    self.log_page_loaded = true;
                } else {
                    self.log_page_events.extend(page.events);
                }
                self.log_next_cursor = page.next_cursor;
            }
            Err(error) => self.last_error = Some(error),
        }
    }

    #[cfg(windows)]
    fn exit_in_progress(&self) -> bool {
        self.exit_state.is_in_progress()
    }

    #[cfg(windows)]
    fn start_exit(&mut self, context: &egui::Context) {
        if self.reject_conflicting_operation() {
            self.show_window(context);
            return;
        }
        if !self.exit_state.begin() {
            return;
        }
        self.last_error = None;
        self.show_window(context);
        let controller = Arc::clone(&self.controller);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = controller
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .shutdown();
            let _ = sender.send(result);
        });
        self.exit_receiver = Some(receiver);
    }

    #[cfg(windows)]
    fn poll_exit(&mut self, context: &egui::Context) {
        let Some(receiver) = self.exit_receiver.as_ref() else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(())) => {
                self.exit_receiver = None;
                self.exit_state.finish();
                context.send_viewport_cmd(ViewportCommand::Close);
            }
            Ok(Err(error)) => {
                self.exit_receiver = None;
                self.exit_state.finish();
                self.last_error = Some(error);
                self.show_window(context);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.exit_receiver = None;
                self.exit_state.finish();
                self.last_error = Some(UiControlError::Operation("退出清理线程意外中断".into()));
                self.show_window(context);
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn confirmation_dialog(&mut self, context: &egui::Context) {
        let Some(action) = self.pending.clone() else {
            return;
        };
        let mut open = true;
        let mut confirm = context.input(|input| input.key_pressed(egui::Key::Enter));
        let mut cancel = context.input(|input| input.key_pressed(egui::Key::Escape));
        egui::Window::new("确认切换")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(context, |ui| {
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("继续切换").clicked() {
                        confirm = true;
                    }
                    if ui.button("取消").clicked() {
                        cancel = true;
                    }
                });
            });
        if confirm {
            match action {
                PendingAction::Proxy(id) => self.request_proxy(id, true),
                PendingAction::Save(draft) => self.save_proxy(draft, true),
                PendingAction::SaveRule(draft) => self.save_rule(draft, true),
                PendingAction::DeleteRule(id) => self.delete_rule(id, true),
                PendingAction::ToggleRule(id, enabled) => self.toggle_rule(id, enabled, true),
            }
        } else if cancel || !open {
            self.pending = None;
            self.last_error = None;
        }
    }

    fn proxy_editor(&mut self, context: &egui::Context) {
        let Some(mut draft) = self.proxy_draft.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new(if draft.id.is_some() {
            "编辑代理"
        } else {
            "新建代理"
        })
        .collapsible(false)
        .resizable(false)
        .min_width(470.0)
        .open(&mut open)
        .show(context, |ui| {
            components::form_row(ui, "名称", Some("用于识别此代理"), |ui| {
                ui.add_sized([290.0, 30.0], egui::TextEdit::singleline(&mut draft.name));
            });
            components::form_row(ui, "协议", None, |ui| {
                egui::ComboBox::from_id_salt("proxy_protocol")
                    .width(290.0)
                    .selected_text(protocol_label(draft.protocol))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.protocol, ProxyProtocol::Socks5, "SOCKS5");
                        ui.selectable_value(&mut draft.protocol, ProxyProtocol::Http, "HTTP");
                    });
            });
            components::form_row(ui, "服务器", Some("IP 地址或域名"), |ui| {
                ui.add_sized([290.0, 30.0], egui::TextEdit::singleline(&mut draft.host));
            });
            components::form_row(ui, "端口", None, |ui| {
                ui.add_sized([110.0, 30.0], egui::TextEdit::singleline(&mut draft.port));
            });
            components::form_row(ui, "认证", None, |ui| {
                ui.checkbox(&mut draft.auth_enabled, "启用用户名和密码");
            });
            if draft.auth_enabled {
                components::form_row(ui, "用户名", None, |ui| {
                    ui.add_sized(
                        [290.0, 30.0],
                        egui::TextEdit::singleline(&mut draft.username),
                    );
                });
                components::form_row(ui, "密码", None, |ui| {
                    ui.add_sized(
                        [290.0, 30.0],
                        egui::TextEdit::singleline(&mut draft.password).password(true),
                    );
                });
            }
            if let Some(UiControlError::Validation { field, message }) = &self.last_error {
                ui.add_space(6.0);
                components::field_error(ui, &format!("{}: {message}", field_label(field)));
            } else if let Some(error) = &self.last_error {
                ui.add_space(6.0);
                components::field_error(ui, &error.to_string());
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                save = ui.button("保存").clicked();
                cancel = ui.button("取消").clicked();
            });
        });
        if save {
            if self.reject_conflicting_operation() {
                self.proxy_draft = Some(draft);
                return;
            }
            let pending = draft.clone();
            let result = self
                .controller
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .save_proxy(draft.clone(), false);
            match result {
                Ok(_) => {
                    wipe_string(&mut draft.password);
                    self.last_error = None;
                    return;
                }
                Err(UiControlError::ConfirmationRequired(message)) => {
                    self.pending = Some(PendingAction::Save(pending));
                    self.last_error = Some(UiControlError::ConfirmationRequired(message));
                    wipe_string(&mut draft.password);
                    return;
                }
                Err(error) => {
                    self.last_error = Some(error);
                }
            }
        }
        if cancel || !open {
            wipe_string(&mut draft.password);
            self.last_error = None;
        } else {
            self.proxy_draft = Some(draft);
        }
    }

    fn save_proxy(&mut self, draft: ProxyDraft, confirmed: bool) {
        if self.reject_conflicting_operation() {
            return;
        }
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .save_proxy(draft, confirmed);
        match result {
            Ok(_) => {
                self.pending = None;
                self.proxy_draft = None;
                self.last_error = None;
            }
            Err(error) => {
                self.pending = None;
                self.last_error = Some(error);
            }
        }
    }

    fn delete_rule(&mut self, id: String, confirmed: bool) {
        if self.reject_conflicting_operation() {
            return;
        }
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .delete_rule(&id, confirmed);
        self.handle_action(result, PendingAction::DeleteRule(id));
    }

    fn toggle_rule(&mut self, id: String, enabled: bool, confirmed: bool) {
        if self.reject_conflicting_operation() {
            return;
        }
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .set_rule_enabled(&id, enabled, confirmed);
        self.handle_action(result, PendingAction::ToggleRule(id, enabled));
    }

    fn save_rule(&mut self, draft: RuleDraft, confirmed: bool) {
        if self.reject_conflicting_operation() {
            return;
        }
        let pending = draft.clone();
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .save_rule(draft, confirmed);
        match result {
            Ok(_) => {
                self.pending = None;
                self.rule_draft = None;
                self.last_error = None;
            }
            Err(UiControlError::ConfirmationRequired(message)) => {
                self.pending = Some(PendingAction::SaveRule(pending));
                self.last_error = Some(UiControlError::ConfirmationRequired(message));
                self.rule_draft = None;
            }
            Err(error) => {
                self.pending = None;
                self.last_error = Some(error);
            }
        }
    }

    fn rule_editor(&mut self, context: &egui::Context) {
        let Some(mut draft) = self.rule_draft.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new(if draft.id.is_some() {
            "编辑规则"
        } else {
            "新建规则"
        })
        .collapsible(false)
        .resizable(false)
        .min_width(500.0)
        .open(&mut open)
        .show(context, |ui| {
            components::form_row(ui, "名称", Some("便于识别规则用途"), |ui| {
                ui.add_sized([300.0, 30.0], egui::TextEdit::singleline(&mut draft.name));
            });
            components::form_row(ui, "启用", None, |ui| {
                ui.checkbox(&mut draft.enabled, "规则生效");
            });
            components::form_row(ui, "目标类型", None, |ui| {
                egui::ComboBox::from_id_salt("rule_target_kind")
                    .width(300.0)
                    .selected_text(rule_target_kind_label(draft.target_kind))
                    .show_ui(ui, |ui| {
                        for kind in [
                            RuleTargetKind::Domain,
                            RuleTargetKind::DomainSuffix,
                            RuleTargetKind::Ip,
                            RuleTargetKind::Cidr,
                            RuleTargetKind::Range,
                        ] {
                            ui.selectable_value(
                                &mut draft.target_kind,
                                kind,
                                rule_target_kind_label(kind),
                            );
                        }
                    });
            });
            components::form_row(
                ui,
                "目标",
                Some("域名、IP、CIDR 或 IP 范围"),
                |ui| {
                    ui.add_sized([300.0, 30.0], egui::TextEdit::singleline(&mut draft.target));
                },
            );
            components::form_row(ui, "端口", Some("例如 443 或 80,443,8000-8080"), |ui| {
                ui.add_sized([300.0, 30.0], egui::TextEdit::singleline(&mut draft.ports));
            });
            components::form_row(ui, "备注", None, |ui| {
                ui.add_sized([300.0, 30.0], egui::TextEdit::singleline(&mut draft.note));
            });
            if let Some(UiControlError::Validation { field, message }) = &self.last_error {
                ui.add_space(6.0);
                components::field_error(ui, &format!("{}: {message}", field_label(field)));
            } else if let Some(error) = &self.last_error {
                ui.add_space(6.0);
                components::field_error(ui, &error.to_string());
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                save = ui.button("保存").clicked();
                cancel = ui.button("取消").clicked();
            });
        });
        if save {
            let result = self
                .controller
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .save_rule(draft.clone(), false);
            match result {
                Ok(_) => {
                    self.last_error = None;
                    return;
                }
                Err(UiControlError::ConfirmationRequired(message)) => {
                    self.pending = Some(PendingAction::SaveRule(draft));
                    self.last_error = Some(UiControlError::ConfirmationRequired(message));
                    return;
                }
                Err(error) => {
                    self.last_error = Some(error);
                }
            }
        }
        if cancel || !open {
            self.last_error = None;
        } else {
            self.rule_draft = Some(draft);
        }
    }

    #[cfg(windows)]
    fn process_tray(&mut self, context: &egui::Context) {
        let commands = self
            .tray
            .as_ref()
            .map(tray::TrayManager::poll)
            .unwrap_or_default();
        for command in commands {
            let requires_window_feedback = command.requires_window_feedback();
            match command {
                tray::TrayCommand::ToggleWindow => {
                    self.window_visible = !self.window_visible;
                    context.send_viewport_cmd(ViewportCommand::Visible(self.window_visible));
                }
                tray::TrayCommand::Mode(mode) => {
                    self.request_mode(mode, false);
                }
                tray::TrayCommand::Profile(value) => match ProfileId::parse(&value) {
                    Ok(id) => {
                        self.request_proxy(id, false);
                    }
                    Err(error) => {
                        self.last_error = Some(UiControlError::Operation(error.to_string()));
                        self.show_window(context);
                    }
                },
                tray::TrayCommand::Page(page) => {
                    self.page = page;
                }
                tray::TrayCommand::Exit => {
                    self.start_exit(context);
                }
            }
            if requires_window_feedback {
                self.show_window(context);
            }
        }
        context.request_repaint_after(std::time::Duration::from_millis(100));
    }

    #[cfg(windows)]
    fn show_window(&mut self, context: &egui::Context) {
        self.window_visible = true;
        context.send_viewport_cmd(ViewportCommand::Visible(true));
        context.send_viewport_cmd(ViewportCommand::Focus);
    }
}

impl eframe::App for DesktopApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = root.ctx().clone();
        self.poll_mode();
        #[cfg(windows)]
        self.process_tray(&context);
        #[cfg(windows)]
        self.poll_exit(&context);
        let exiting = {
            #[cfg(windows)]
            {
                self.exit_in_progress()
            }
            #[cfg(not(windows))]
            {
                false
            }
        };
        let state = (!exiting).then(|| {
            let state = self.state();
            self.state_cache = state.clone();
            self.mode_switch_job.update_snapshot(state.clone());
            state
        });
        #[cfg(windows)]
        if let (Some(state), Some(tray)) = (state.as_ref(), &mut self.tray)
            && let Err(error) = tray.sync(state, self.mode_switch_job.is_running())
        {
            self.last_error = Some(UiControlError::Operation(error));
        }
        if !exiting && context.input(|input| input.viewport().close_requested()) {
            context.send_viewport_cmd(ViewportCommand::CancelClose);
            context.send_viewport_cmd(ViewportCommand::Visible(false));
            #[cfg(windows)]
            {
                self.window_visible = false;
            }
        }
        if exiting {
            egui::CentralPanel::default()
                .frame(egui::Frame::central_panel(root.style()).fill(theme::CANVAS))
                .show(root, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(120.0);
                        egui::Frame::new()
                            .fill(theme::SURFACE)
                            .stroke(Stroke::new(1.0, theme::BORDER))
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::same(24))
                            .show(ui, |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.add(egui::Spinner::new().size(24.0).color(theme::BLUE));
                                    ui.add_space(10.0);
                                    ui.label(RichText::new("正在退出").size(20.0).strong());
                                    ui.label(
                                        RichText::new("正在停止内核并恢复网络设置，请勿关闭程序。")
                                            .color(theme::MUTED),
                                    );
                                });
                            });
                    });
                });
            return;
        }
        let state = state.expect("state exists unless exiting");
        self.navigation(root);
        self.status_bar(root, &state);
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(root.style()).inner_margin(egui::Margin::same(28)))
            .show(root, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Status => self.status_page(ui, &state),
                        Page::Proxies => self.proxies_page(ui, &state),
                        Page::Rules => self.rules_page(ui, &state),
                        Page::Logs => self.logs_page(ui, &state),
                        Page::Settings => self.settings_page(ui, &state),
                    });
            });
        self.confirmation_dialog(&context);
        self.proxy_editor(&context);
        self.rule_editor(&context);
        #[cfg(windows)]
        self.import_preview_dialog(&context, &state);
    }
}

fn configure_fonts(context: &egui::Context) {
    let candidates = if cfg!(windows) {
        vec![
            "C:\\Windows\\Fonts\\msyh.ttc",
            "C:\\Windows\\Fonts\\msyhbd.ttc",
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
        ]
    } else {
        vec![
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        ]
    };
    let Some(bytes) = candidates.iter().find_map(|path| std::fs::read(path).ok()) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "system-cjk".into(),
        egui::FontData::from_owned(bytes).into(),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "system-cjk".into());
    }
    context.set_fonts(fonts);
}

fn detail_grid<const N: usize>(ui: &mut egui::Ui, rows: [(&str, String); N]) {
    let id = ui.id().with(("detail_grid", rows.first().map(|row| row.0)));
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([18.0, 8.0])
        .show(ui, |ui| {
            for (label, value) in rows {
                detail_row(ui, label, &value);
            }
        });
}

fn mode_label(mode: RoutingMode) -> &'static str {
    pages::status::mode_label(mode)
}

fn is_applying(state: &UiState) -> bool {
    pages::status::is_applying(state)
}

fn global_phase_label(phase: crate::core::SwitchPhase) -> &'static str {
    pages::status::global_phase_label(phase)
}

fn outbound_label(outbound: crate::logs::ConnectionOutbound) -> &'static str {
    pages::logs::outbound_label(outbound)
}

fn rule_matches(rule: &crate::domain::RoutingRule, filter: &str) -> bool {
    pages::rules::matches(rule, filter)
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).weak());
    ui.label(value);
    ui.end_row();
}

fn table_header(ui: &mut egui::Ui, cells: &[&str]) {
    ui.horizontal(|ui| {
        for cell in cells {
            ui.add_sized(
                [132.0, 28.0],
                egui::Label::new(RichText::new(*cell).strong()),
            );
        }
    });
    ui.separator();
}

fn log_detail(ui: &mut egui::Ui, event: &crate::logs::ConnectionEvent) {
    let values = log_detail_values(event);
    egui::Grid::new(("log_detail", event.connection_id))
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            for (label, value) in &values {
                log_detail_row(ui, label, value);
            }
        });
}

fn log_detail_values(event: &crate::logs::ConnectionEvent) -> [(&'static str, String); 6] {
    pages::logs::detail_values(event, event_time)
}

fn log_detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).weak());
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::Label::new(value).wrap().selectable(true));
        if ui
            .small_button("复制")
            .on_hover_text(format!("复制{label}"))
            .clicked()
        {
            ui.ctx().copy_text(value.to_owned());
        }
    });
    ui.end_row();
}

fn clear_log_selection(selection: &mut Option<u64>) {
    *selection = None;
}

fn mode_switch_conflict_error() -> UiControlError {
    UiControlError::Operation("正在切换代理模式，请等待当前操作完成".into())
}

fn has_unread_log_events(
    recent_events: &[crate::logs::ConnectionEvent],
    page_events: &[crate::logs::ConnectionEvent],
) -> bool {
    recent_events.last().is_some_and(|latest| {
        !page_events
            .iter()
            .any(|event| event.connection_id == latest.connection_id)
    })
}

fn phase_label(phase: crate::core::SwitchPhase) -> &'static str {
    match phase {
        crate::core::SwitchPhase::Direct => "直连",
        crate::core::SwitchPhase::Running => "运行中",
        crate::core::SwitchPhase::Reconfiguring => "正在应用",
        crate::core::SwitchPhase::Error => "异常",
    }
}

fn event_time(unix_ms: u128) -> String {
    let seconds = (unix_ms / 1_000) % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3_600,
        (seconds / 60) % 60,
        seconds % 60
    )
}

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitState {
    Idle,
    InProgress,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl ExitState {
    fn begin(&mut self) -> bool {
        if self.is_in_progress() {
            return false;
        }
        *self = Self::InProgress;
        true
    }

    fn finish(&mut self) {
        *self = Self::Idle;
    }

    fn is_in_progress(self) -> bool {
        matches!(self, Self::InProgress)
    }
}

fn protocol_label(protocol: ProxyProtocol) -> &'static str {
    pages::proxies::protocol_label(protocol)
}

fn host_label(host: &crate::domain::ProxyHost) -> String {
    pages::proxies::host_label(host)
}

fn rule_target_kind_label(kind: RuleTargetKind) -> &'static str {
    match kind {
        RuleTargetKind::Domain => "精确域名",
        RuleTargetKind::DomainSuffix => "域名及子域名",
        RuleTargetKind::Ip => "单个 IP",
        RuleTargetKind::Cidr => "CIDR",
        RuleTargetKind::Range => "IP 范围",
    }
}

fn rule_target_label(target: &crate::domain::RuleTarget) -> String {
    pages::rules::target_label(target)
}

fn rule_ports_label(ports: &crate::domain::PortSet) -> String {
    pages::rules::ports_label(ports)
}

fn field_label(field: &str) -> &'static str {
    match field {
        "name" => "名称",
        "host" => "服务器",
        "port" => "端口",
        "credentials" => "认证",
        "active_proxy" => "当前代理",
        "mode" => "代理模式",
        "target" => "目标",
        "ports" => "端口",
        "rule" => "规则",
        _ => "字段",
    }
}

fn wipe_string(value: &mut String) {
    unsafe { value.as_bytes_mut().fill(0) };
    value.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_has_exactly_the_five_required_pages() {
        assert_eq!(Page::ALL.len(), 5);
        assert_eq!(
            Page::ALL.map(Page::label),
            ["状态", "代理", "分流规则", "连接日志", "设置"]
        );
    }

    #[test]
    fn navigation_uses_stable_wide_and_compact_layouts_at_the_breakpoint() {
        assert_eq!(
            navigation_layout(theme::NAVIGATION_COMPACT_BREAKPOINT).width,
            theme::NAVIGATION_WIDE_WIDTH
        );
        assert!(!navigation_layout(theme::NAVIGATION_COMPACT_BREAKPOINT).compact);
        assert_eq!(
            navigation_layout(theme::NAVIGATION_COMPACT_BREAKPOINT - 1.0).width,
            theme::NAVIGATION_COMPACT_WIDTH
        );
        assert!(navigation_layout(theme::NAVIGATION_COMPACT_BREAKPOINT - 1.0).compact);
    }

    #[test]
    fn every_navigation_page_has_an_embedded_local_icon_resource() {
        assert_eq!(
            Page::ALL.map(Page::icon_resource_name),
            [
                "status.png",
                "proxies.png",
                "rules.png",
                "logs.png",
                "settings.png"
            ]
        );
        for page in Page::ALL {
            let _ = page.icon();
        }
    }

    #[test]
    fn connection_log_detail_preserves_long_redacted_values() {
        let long = "proxy.example.test: authentication failed after the upstream returned a detailed diagnostic that remains readable";
        let event = crate::logs::ConnectionEvent {
            observed_at_unix_ms: 1_000,
            connection_id: 7,
            target: "very-long-target.example.test".into(),
            port: 443,
            outbound: crate::logs::ConnectionOutbound::Proxy,
            rule: crate::logs::RuleAttribution::Known("very-long-rule-name".into()),
            result: crate::logs::ConnectionResult::Failure(long.into()),
            config_revision: 1,
        };
        let values = log_detail_values(&event);
        assert_eq!(values[1], ("目标", "very-long-target.example.test".into()));
        assert_eq!(values[3], ("出站", "代理".into()));
        assert_eq!(values[4], ("规则", "very-long-rule-name".into()));
        assert_eq!(values[5], ("结果", long.into()));
    }

    #[test]
    fn connection_log_detail_handles_success_and_unknown_rule() {
        let event = crate::logs::ConnectionEvent {
            observed_at_unix_ms: 86_401_000,
            connection_id: 8,
            target: "example.test".into(),
            port: 80,
            outbound: crate::logs::ConnectionOutbound::Unknown,
            rule: crate::logs::RuleAttribution::Unknown,
            result: crate::logs::ConnectionResult::Success,
            config_revision: 1,
        };
        let values = log_detail_values(&event);
        assert_eq!(values[0], ("时间", "00:00:01".into()));
        assert_eq!(values[3], ("出站", "未知".into()));
        assert_eq!(values[4], ("规则", "未知".into()));
        assert_eq!(values[5], ("结果", "成功".into()));
    }

    #[test]
    fn connection_log_detail_redacts_injected_failure_text() {
        let event = crate::logs::ConnectionEvent {
            observed_at_unix_ms: 0,
            connection_id: 9,
            target: "example.test".into(),
            port: 443,
            outbound: crate::logs::ConnectionOutbound::Proxy,
            rule: crate::logs::RuleAttribution::Unknown,
            result: crate::logs::ConnectionResult::Failure(
                "connect failed: password=not-for-display".into(),
            ),
            config_revision: 1,
        };
        let values = log_detail_values(&event);
        assert_eq!(
            values[5],
            ("结果", "connect failed: password=[REDACTED]".into())
        );
    }

    #[test]
    fn exit_state_starts_one_transaction_and_recovers_after_completion() {
        let mut state = ExitState::Idle;
        assert!(state.begin());
        assert!(state.is_in_progress());
        assert!(!state.begin());
        state.finish();
        assert!(!state.is_in_progress());
        assert!(state.begin());
    }

    #[test]
    fn clearing_logs_closes_the_expanded_detail() {
        let mut selection = Some(17);
        clear_log_selection(&mut selection);
        assert_eq!(selection, None);
    }

    #[test]
    fn unread_log_hint_only_appears_for_an_event_missing_from_the_loaded_page() {
        let event = crate::logs::ConnectionEvent {
            observed_at_unix_ms: 0,
            connection_id: 17,
            target: "example.test".into(),
            port: 443,
            outbound: crate::logs::ConnectionOutbound::Proxy,
            rule: crate::logs::RuleAttribution::Unknown,
            result: crate::logs::ConnectionResult::Success,
            config_revision: 1,
        };
        assert!(!has_unread_log_events(&[event.clone()], &[event.clone()]));
        assert!(has_unread_log_events(&[event], &[]));
        assert!(!has_unread_log_events(&[], &[]));
    }

    #[test]
    fn conflicting_operations_report_a_readable_mode_switch_error() {
        assert_eq!(
            mode_switch_conflict_error().to_string(),
            "正在切换代理模式，请等待当前操作完成"
        );
    }

    #[test]
    fn routing_mode_labels_remain_explicit() {
        assert_eq!(mode_label(RoutingMode::Rules), "规则代理");
        assert_eq!(mode_label(RoutingMode::GlobalProxy), "全局代理");
        assert_eq!(mode_label(RoutingMode::Direct), "全局直连");
    }

    #[test]
    fn global_status_labels_cover_every_runtime_phase() {
        assert_eq!(
            global_phase_label(crate::core::SwitchPhase::Direct),
            "内核未运行"
        );
        assert_eq!(
            global_phase_label(crate::core::SwitchPhase::Running),
            "内核运行中"
        );
        assert_eq!(
            global_phase_label(crate::core::SwitchPhase::Reconfiguring),
            "正在应用"
        );
        assert_eq!(
            global_phase_label(crate::core::SwitchPhase::Error),
            "运行异常"
        );
    }

    #[test]
    fn rule_filter_matches_name_target_and_note_without_mutating_rule() {
        let rule = crate::domain::RoutingRule {
            id: "office-gateway".into(),
            name: "公司内网".into(),
            enabled: true,
            target: crate::domain::RuleTarget::Domain(
                crate::domain::DomainName::parse("gateway.example.com").unwrap(),
            ),
            ports: "443".parse().unwrap(),
            note: "VPN 入口".into(),
        };
        assert!(rule_matches(&rule, "公司"));
        assert!(rule_matches(&rule, "GATEWAY"));
        assert!(rule_matches(&rule, "vpn"));
        assert!(!rule_matches(&rule, "database"));
        assert_eq!(rule.name, "公司内网");
        assert_eq!(rule.note, "VPN 入口");
    }
}
