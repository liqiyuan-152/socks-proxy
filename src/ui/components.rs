use eframe::egui::{self, Align, Color32, FontId, Layout, Response, RichText, Stroke};

use super::theme;

pub fn page_header(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    action: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new(title)
                    .font(FontId::proportional(22.0))
                    .strong()
                    .color(theme::TEXT),
            );
            ui.label(RichText::new(subtitle).small().color(theme::MUTED));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), action);
    });
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(10.0);
}

pub fn section(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(title).strong().color(theme::TEXT));
    ui.add_space(6.0);
    let content_width = (ui.available_width() - 28.0).max(0.0);
    egui::Frame::new()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(theme::BORDER_WIDTH, theme::BORDER))
        .corner_radius(egui::CornerRadius::same(theme::SECTION_CORNER_RADIUS))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_min_width(content_width);
            content(ui);
        });
}

pub fn state_badge(ui: &mut egui::Ui, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        ui.colored_label(color, "●");
        ui.label(RichText::new(text).color(theme::TEXT));
    });
}

pub fn icon_button(
    ui: &mut egui::Ui,
    icon: egui::Image<'static>,
    accessible_name: &str,
    size: egui::Vec2,
    enabled: bool,
    selected: bool,
) -> Response {
    ui.add_enabled_ui(enabled, |ui| {
        ui.add_sized(size, egui::Button::image(icon).selected(selected))
    })
    .inner
    .on_hover_text(accessible_name)
}

pub fn error_banner(ui: &mut egui::Ui, message: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(255, 241, 242))
        .stroke(Stroke::new(1.0, Color32::from_rgb(254, 202, 202)))
        .corner_radius(egui::CornerRadius::same(theme::CONTROL_CORNER_RADIUS))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.colored_label(theme::RED, format!("错误: {message}"));
        });
}

pub fn success_banner(ui: &mut egui::Ui, message: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(240, 253, 244))
        .stroke(Stroke::new(1.0, Color32::from_rgb(187, 247, 208)))
        .corner_radius(egui::CornerRadius::same(theme::CONTROL_CORNER_RADIUS))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.colored_label(theme::GREEN, format!("成功: {message}"));
        });
}

pub fn operation_banner(ui: &mut egui::Ui, color: Color32, title: &str, message: &str) {
    egui::Frame::new()
        .fill(theme::BLUE_SOFT)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(egui::CornerRadius::same(theme::CONTROL_CORNER_RADIUS))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(color, "●");
                ui.label(RichText::new(title).strong().color(theme::TEXT));
                ui.label(RichText::new(message).color(theme::MUTED));
            });
        });
}

pub fn empty_state(ui: &mut egui::Ui, message: &str) {
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(message).color(theme::MUTED));
    });
}

pub fn form_row(
    ui: &mut egui::Ui,
    label: &str,
    help: Option<&str>,
    field: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal_top(|ui| {
        ui.set_min_width(92.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(label).color(theme::TEXT));
            if let Some(help) = help {
                ui.label(RichText::new(help).small().color(theme::MUTED));
            }
        });
        ui.vertical(|ui| field(ui));
    });
    ui.add_space(5.0);
}

pub fn field_error(ui: &mut egui::Ui, message: &str) {
    ui.colored_label(theme::RED, format!("! {message}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_icon_button(enabled: bool) -> Response {
        let context = egui::Context::default();
        context.begin_pass(egui::RawInput::default());
        let mut ui = egui::Ui::new(
            context.clone(),
            egui::Id::new("icon_button_test"),
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(200.0, 100.0),
            )),
        );
        let response = icon_button(
            &mut ui,
            egui::Image::new(egui::include_image!("../../assets/lucide/status.png")),
            "状态",
            egui::Vec2::new(64.0, theme::NAVIGATION_ROW_HEIGHT),
            enabled,
            false,
        );
        let mut output = context.end_pass();
        output.textures_delta.clear();
        response
    }

    #[test]
    fn icon_button_preserves_a_stable_row_size_when_enabled_or_disabled() {
        let enabled = render_icon_button(true);
        let disabled = render_icon_button(false);
        assert!(enabled.enabled());
        assert!(!disabled.enabled());
        assert_eq!(
            enabled.rect.size(),
            egui::Vec2::new(64.0, theme::NAVIGATION_ROW_HEIGHT)
        );
        assert_eq!(disabled.rect.size(), enabled.rect.size());
    }
}
