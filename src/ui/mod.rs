mod controller;
#[cfg(windows)]
mod tray;
pub use controller::{
    BackupPreview, DirectOnlyRuntime, ManagedDesktopController, MemoryCredentialVault, ProxyDraft,
    RuleDraft, RuleTargetKind, SharedController, UiControlError, UiState,
};

use eframe::egui::{self, Align, Color32, FontId, Layout, RichText, Stroke, Vec2, ViewportCommand};
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
    #[cfg(windows)]
    window_visible: bool,
    exiting: bool,
    #[cfg(windows)]
    tray: Option<tray::TrayManager>,
}

#[derive(Clone)]
enum PendingAction {
    Mode(RoutingMode),
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
        configure_style(&context.egui_ctx);
        let app = Self {
            page: Page::Status,
            controller,
            proxy_draft: None,
            rule_draft: None,
            #[cfg(windows)]
            pending_import: None,
            pending: None,
            last_error: None,
            #[cfg(windows)]
            window_visible: true,
            exiting: false,
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
        egui::Panel::left("navigation")
            .exact_size(176.0)
            .resizable(false)
            .frame(
                egui::Frame::side_top_panel(root.style())
                    .fill(Color32::from_rgb(245, 246, 248))
                    .inner_margin(egui::Margin::symmetric(14, 16)),
            )
            .show(root, |ui| {
                ui.label(
                    RichText::new("Socks Proxy")
                        .font(FontId::proportional(19.0))
                        .strong(),
                );
                ui.add_space(20.0);
                for page in Page::ALL {
                    let selected = self.page == page;
                    if ui
                        .add_sized(
                            [148.0, 36.0],
                            egui::Button::selectable(selected, page.label()),
                        )
                        .clicked()
                    {
                        self.page = page;
                    }
                    ui.add_space(4.0);
                }
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.label(RichText::new("Windows 10/11 x64").small().weak());
                });
            });
    }

    fn state(&self) -> UiState {
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
                    ui.colored_label(color, "●");
                    ui.label(text);
                    ui.separator();
                    ui.label(if state.runtime.traffic_may_be_direct {
                        "代理接管失效，流量可能直连"
                    } else {
                        "运行状态已同步"
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(match state.runtime.phase {
                                crate::core::SwitchPhase::Direct => "内核未运行",
                                crate::core::SwitchPhase::Running => "内核运行中",
                                crate::core::SwitchPhase::Reconfiguring => "正在应用",
                                crate::core::SwitchPhase::Error => "运行异常",
                            })
                            .weak(),
                        );
                    });
                });
            });
    }

    fn status_page(&mut self, ui: &mut egui::Ui, state: &UiState) {
        page_heading(ui, "状态", "当前网络模式和实际运行状态");
        ui.add_space(18.0);
        ui.label(RichText::new("代理模式").strong());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for (mode, label) in [
                (RoutingMode::Direct, "全局直连"),
                (RoutingMode::Rules, "规则代理"),
                (RoutingMode::GlobalProxy, "全局代理"),
            ] {
                if ui
                    .selectable_label(state.runtime.applied_mode == Some(mode), label)
                    .clicked()
                    && state.runtime.applied_mode != Some(mode)
                {
                    self.request_mode(mode, false);
                }
            }
        });
        self.error_banner(ui);
        ui.add_space(22.0);
        ui.label(RichText::new("当前代理").strong());
        ui.add_space(6.0);
        let active = state.config.profiles.active();
        egui::ComboBox::from_id_salt("active_proxy")
            .width(320.0)
            .selected_text(active.map_or("未选择", |profile| profile.name.as_str()))
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
        ui.add_space(28.0);
        egui::Grid::new("status_details")
            .num_columns(2)
            .spacing([36.0, 12.0])
            .show(ui, |ui| {
                detail_row(ui, "运行状态", phase_label(state.runtime.phase));
                detail_row(
                    ui,
                    "已应用配置",
                    &format!("修订 {}", state.runtime.applied_revision),
                );
                detail_row(
                    ui,
                    "DNS",
                    if state.runtime.applied_mode == Some(RoutingMode::Direct) {
                        "系统 DNS"
                    } else {
                        "受管理"
                    },
                );
                detail_row(
                    ui,
                    "最近错误",
                    state.runtime.failure.as_deref().unwrap_or("无"),
                );
            });
    }

    fn proxies_page(&mut self, ui: &mut egui::Ui, state: &UiState) {
        ui.horizontal(|ui| {
            ui.heading("代理");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("+ 新建代理").clicked() {
                    self.last_error = None;
                    self.proxy_draft = Some(ProxyDraft::default());
                }
            });
        });
        self.error_banner(ui);
        ui.add_space(16.0);
        egui::Grid::new("proxy_table")
            .num_columns(7)
            .spacing([20.0, 10.0])
            .striped(true)
            .show(ui, |ui| {
                for heading in ["名称", "协议", "服务器", "认证", "状态", "", ""] {
                    ui.label(RichText::new(heading).strong());
                }
                ui.end_row();
                for profile in state.config.profiles.iter() {
                    ui.label(&profile.name);
                    ui.label(protocol_label(profile.protocol));
                    ui.label(format!("{}:{}", host_label(&profile.host), profile.port));
                    ui.label(if profile.auth_enabled {
                        "已配置"
                    } else {
                        "无"
                    });
                    ui.label(if state.config.profiles.active_id() == Some(&profile.id) {
                        "当前"
                    } else {
                        ""
                    });
                    if ui.small_button("编辑").clicked() {
                        self.last_error = None;
                        self.proxy_draft = Some(ProxyDraft::from_profile(profile));
                    }
                    if ui.small_button("删除").clicked() {
                        self.delete_proxy(&profile.id);
                    }
                    ui.end_row();
                }
            });
        if state.config.profiles.iter().next().is_none() {
            ui.add_space(28.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("暂无代理").weak()));
        }
    }

    fn rules_page(&mut self, ui: &mut egui::Ui, state: &UiState) {
        ui.horizontal(|ui| {
            ui.heading("分流规则");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("+ 新建规则").clicked() {
                    self.last_error = None;
                    self.rule_draft = Some(RuleDraft::default());
                }
            });
        });
        self.error_banner(ui);
        ui.add_space(16.0);
        egui::Grid::new("rules_table")
            .num_columns(7)
            .spacing([18.0, 10.0])
            .striped(true)
            .show(ui, |ui| {
                for heading in ["启用", "名称", "目标", "端口", "备注", "", ""] {
                    ui.label(RichText::new(heading).strong());
                }
                ui.end_row();
                for rule in state.config.rules.clone() {
                    let mut enabled = rule.enabled;
                    if ui.checkbox(&mut enabled, "").changed() {
                        self.toggle_rule(rule.id.clone(), enabled, false);
                    }
                    ui.label(&rule.name);
                    ui.label(rule_target_label(&rule.target));
                    ui.label(rule_ports_label(&rule.ports));
                    ui.label(&rule.note);
                    if ui.small_button("编辑").clicked() {
                        self.last_error = None;
                        self.rule_draft = Some(RuleDraft::from_rule(&rule));
                    }
                    if ui.small_button("删除").clicked() {
                        self.delete_rule(rule.id.clone(), false);
                    }
                    ui.end_row();
                }
            });
        if state.config.rules.is_empty() {
            ui.add_space(28.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("暂无规则").weak()));
        }
    }

    fn logs_page(&mut self, ui: &mut egui::Ui, state: &UiState) {
        ui.horizontal(|ui| {
            ui.heading("连接日志");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !state.connection_events.is_empty(),
                        egui::Button::new("清空"),
                    )
                    .clicked()
                {
                    let result = self
                        .controller
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .clear_logs();
                    self.last_error = result.err();
                }
            });
        });
        self.error_banner(ui);
        ui.add_space(16.0);
        table_header(ui, &["时间", "目标", "出站", "规则", "结果"]);
        if state.connection_events.is_empty() {
            ui.add_space(28.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("暂无连接记录").weak()));
            return;
        }
        egui::ScrollArea::both().show(ui, |ui| {
            for event in state.connection_events.iter().rev() {
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
                        ui.add_sized([132.0, 28.0], egui::Label::new(value).truncate());
                    }
                });
                ui.separator();
            }
        });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui, state: &UiState) {
        page_heading(ui, "设置", "启动、备份与网络恢复");
        ui.add_space(18.0);
        let mut startup_enabled = state.config.preferences.start_with_windows;
        ui.add_enabled(false, egui::Checkbox::new(&mut startup_enabled, "开机启动"));
        ui.label(
            RichText::new("首版默认关闭，当前仅展示已保存偏好")
                .small()
                .weak(),
        );
        ui.add_space(12.0);
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
                            self.last_error =
                                Some(UiControlError::Operation(format!("读取备份失败: {error}")))
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
        self.error_banner(ui);
        ui.add_space(24.0);
        ui.label(RichText::new("网络恢复").strong());
        ui.add_space(6.0);
        ui.label("没有待恢复的网络修改");
    }

    #[cfg(windows)]
    fn import_preview_dialog(&mut self, context: &egui::Context, state: &UiState) {
        let Some((_, preview)) = self.pending_import.as_ref() else {
            return;
        };
        let preview = *preview;
        let direct = state.runtime.phase == crate::core::SwitchPhase::Direct
            && state.runtime.applied_mode == Some(RoutingMode::Direct);
        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("确认导入备份")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(context, |ui| {
                ui.label("导入将整体替换当前代理、规则和启动偏好。");
                ui.add_space(10.0);
                egui::Grid::new("backup_import_preview")
                    .num_columns(2)
                    .spacing([24.0, 8.0])
                    .show(ui, |ui| {
                        detail_row(ui, "代理", &preview.profiles.to_string());
                        detail_row(ui, "规则", &preview.rules.to_string());
                        detail_row(ui, "待补充认证", &preview.missing_credentials.to_string());
                    });
                if preview.missing_credentials > 0 {
                    ui.add_space(8.0);
                    ui.colored_label(
                        Color32::from_rgb(151, 100, 22),
                        "备份不包含密码；导入后需为这些代理重新填写认证信息。",
                    );
                }
                if !direct {
                    ui.add_space(8.0);
                    ui.colored_label(
                        Color32::from_rgb(181, 55, 55),
                        "请先切换到全局直连并完成网络恢复，再提交导入。",
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
                self.last_error = result.err();
            }
        } else if cancel || !open {
            self.pending_import = None;
        }
    }

    fn request_mode(&mut self, mode: RoutingMode, confirmed: bool) {
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .switch_mode(mode, confirmed);
        self.handle_action(result, PendingAction::Mode(mode));
    }

    fn request_proxy(&mut self, id: ProfileId, confirmed: bool) {
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .select_proxy(&id, confirmed);
        self.handle_action(result, PendingAction::Proxy(id));
    }

    fn delete_proxy(&mut self, id: &ProfileId) {
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .delete_proxy(id);
        self.last_error = result.err();
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
            ui.colored_label(Color32::from_rgb(181, 55, 55), error.to_string());
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
                ui.label("切换需要重启网络内核，现有 SSH 等连接可能中断。");
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
                PendingAction::Mode(mode) => self.request_mode(mode, true),
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
        .open(&mut open)
        .show(context, |ui| {
            egui::Grid::new("proxy_editor_fields")
                .num_columns(2)
                .spacing([16.0, 10.0])
                .show(ui, |ui| {
                    ui.label("名称");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.end_row();
                    ui.label("协议");
                    egui::ComboBox::from_id_salt("proxy_protocol")
                        .selected_text(protocol_label(draft.protocol))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut draft.protocol,
                                ProxyProtocol::Socks5,
                                "SOCKS5",
                            );
                            ui.selectable_value(&mut draft.protocol, ProxyProtocol::Http, "HTTP");
                        });
                    ui.end_row();
                    ui.label("服务器");
                    ui.text_edit_singleline(&mut draft.host);
                    ui.end_row();
                    ui.label("端口");
                    ui.text_edit_singleline(&mut draft.port);
                    ui.end_row();
                    ui.label("认证");
                    ui.checkbox(&mut draft.auth_enabled, "启用");
                    ui.end_row();
                    if draft.auth_enabled {
                        ui.label("用户名");
                        ui.text_edit_singleline(&mut draft.username);
                        ui.end_row();
                        ui.label("密码");
                        ui.add(egui::TextEdit::singleline(&mut draft.password).password(true));
                        ui.end_row();
                    }
                });
            if let Some(UiControlError::Validation { field, message }) = &self.last_error {
                ui.add_space(6.0);
                ui.colored_label(
                    Color32::from_rgb(181, 55, 55),
                    format!("{}: {message}", field_label(field)),
                );
            } else if let Some(error) = &self.last_error {
                ui.add_space(6.0);
                ui.colored_label(Color32::from_rgb(181, 55, 55), error.to_string());
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                save = ui.button("保存").clicked();
                cancel = ui.button("取消").clicked();
            });
        });
        if save {
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
                Err(error) => self.last_error = Some(error),
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
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .save_proxy(draft, confirmed);
        match result {
            Ok(_) => {
                self.pending = None;
                self.last_error = None;
                self.proxy_draft = None;
            }
            Err(error) => {
                self.pending = None;
                self.last_error = Some(error);
            }
        }
    }

    fn delete_rule(&mut self, id: String, confirmed: bool) {
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .delete_rule(&id, confirmed);
        self.handle_action(result, PendingAction::DeleteRule(id));
    }

    fn toggle_rule(&mut self, id: String, enabled: bool, confirmed: bool) {
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .set_rule_enabled(&id, enabled, confirmed);
        self.handle_action(result, PendingAction::ToggleRule(id, enabled));
    }

    fn save_rule(&mut self, draft: RuleDraft, confirmed: bool) {
        let pending = draft.clone();
        let result = self
            .controller
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .save_rule(draft, confirmed);
        match result {
            Ok(_) => {
                self.pending = None;
                self.last_error = None;
                self.rule_draft = None;
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
        .open(&mut open)
        .show(context, |ui| {
            egui::Grid::new("rule_editor_fields")
                .num_columns(2)
                .spacing([16.0, 10.0])
                .show(ui, |ui| {
                    ui.label("名称");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.end_row();
                    ui.label("启用");
                    ui.checkbox(&mut draft.enabled, "");
                    ui.end_row();
                    ui.label("目标类型");
                    egui::ComboBox::from_id_salt("rule_target_kind")
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
                    ui.end_row();
                    ui.label("目标");
                    ui.text_edit_singleline(&mut draft.target);
                    ui.end_row();
                    ui.label("端口");
                    ui.text_edit_singleline(&mut draft.ports);
                    ui.end_row();
                    ui.label("备注");
                    ui.text_edit_singleline(&mut draft.note);
                    ui.end_row();
                });
            if let Some(UiControlError::Validation { field, message }) = &self.last_error {
                ui.add_space(6.0);
                ui.colored_label(
                    Color32::from_rgb(181, 55, 55),
                    format!("{}: {message}", field_label(field)),
                );
            } else if let Some(error) = &self.last_error {
                ui.add_space(6.0);
                ui.colored_label(Color32::from_rgb(181, 55, 55), error.to_string());
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
                Err(error) => self.last_error = Some(error),
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
            match command {
                tray::TrayCommand::ToggleWindow => {
                    self.window_visible = !self.window_visible;
                    context.send_viewport_cmd(ViewportCommand::Visible(self.window_visible));
                }
                tray::TrayCommand::Mode(mode) => {
                    self.request_mode(mode, false);
                    self.show_window(context);
                }
                tray::TrayCommand::Profile(value) => match ProfileId::parse(&value) {
                    Ok(id) => {
                        self.request_proxy(id, false);
                        self.show_window(context);
                    }
                    Err(error) => {
                        self.last_error = Some(UiControlError::Operation(error.to_string()))
                    }
                },
                tray::TrayCommand::Page(page) => {
                    self.page = page;
                    self.show_window(context);
                }
                tray::TrayCommand::Exit => {
                    let result = self
                        .controller
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .shutdown();
                    match result {
                        Ok(()) => {
                            self.exiting = true;
                            context.send_viewport_cmd(ViewportCommand::Close);
                        }
                        Err(error) => {
                            self.last_error = Some(error);
                            self.show_window(context);
                        }
                    }
                }
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
        #[cfg(windows)]
        self.process_tray(&context);
        let state = self.state();
        #[cfg(windows)]
        if let Some(tray) = &mut self.tray
            && let Err(error) = tray.sync(&state)
        {
            self.last_error = Some(UiControlError::Operation(error));
        }
        if !self.exiting && context.input(|input| input.viewport().close_requested()) {
            context.send_viewport_cmd(ViewportCommand::CancelClose);
            context.send_viewport_cmd(ViewportCommand::Visible(false));
            #[cfg(windows)]
            {
                self.window_visible = false;
            }
        }
        self.navigation(root);
        self.status_bar(root, &state);
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(root.style()).inner_margin(egui::Margin::same(28)))
            .show(root, |ui| match self.page {
                Page::Status => self.status_page(ui, &state),
                Page::Proxies => self.proxies_page(ui, &state),
                Page::Rules => self.rules_page(ui, &state),
                Page::Logs => self.logs_page(ui, &state),
                Page::Settings => self.settings_page(ui, &state),
            });
        self.confirmation_dialog(&context);
        self.proxy_editor(&context);
        self.rule_editor(&context);
        #[cfg(windows)]
        self.import_preview_dialog(&context, &state);
    }
}

fn configure_style(context: &egui::Context) {
    let mut style = (*context.global_style()).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 7.0);
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = Color32::from_rgb(252, 252, 253);
    style.visuals.window_corner_radius = egui::CornerRadius::same(6);
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(4);
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(4);
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(4);
    context.set_global_style(style);
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

fn page_heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.heading(title);
    ui.label(RichText::new(subtitle).weak());
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

fn protocol_label(protocol: ProxyProtocol) -> &'static str {
    match protocol {
        ProxyProtocol::Socks5 => "SOCKS5",
        ProxyProtocol::Http => "HTTP",
    }
}

fn host_label(host: &crate::domain::ProxyHost) -> String {
    match host {
        crate::domain::ProxyHost::Ip(ip) => ip.to_string(),
        crate::domain::ProxyHost::Domain(domain) => domain.as_str().to_owned(),
    }
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
    match target {
        crate::domain::RuleTarget::Domain(value) => value.as_str().to_owned(),
        crate::domain::RuleTarget::DomainSuffix(value) => format!("*.{}", value.as_str()),
        crate::domain::RuleTarget::Ip(value) => value.to_string(),
        crate::domain::RuleTarget::Cidr(value) => value.to_string(),
        crate::domain::RuleTarget::Range(value) => value.to_string(),
    }
}

fn rule_ports_label(ports: &crate::domain::PortSet) -> String {
    if ports.intervals() == [(1, u16::MAX)] {
        return "全部".into();
    }
    ports
        .intervals()
        .iter()
        .map(|(start, end)| {
            if start == end {
                start.to_string()
            } else {
                format!("{start}-{end}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
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
}
