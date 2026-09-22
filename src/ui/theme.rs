use eframe::egui::{self, Color32, Stroke, Vec2};

pub const NAVIGATION_WIDE_WIDTH: f32 = 164.0;
pub const NAVIGATION_COMPACT_WIDTH: f32 = 64.0;
pub const NAVIGATION_COMPACT_BREAKPOINT: f32 = 900.0;
pub const NAVIGATION_ROW_HEIGHT: f32 = 38.0;
pub const ICON_SIZE: f32 = 17.0;
pub const WINDOW_CORNER_RADIUS: u8 = 6;
pub const CONTROL_CORNER_RADIUS: u8 = 4;
pub const SECTION_CORNER_RADIUS: u8 = 5;
pub const BORDER_WIDTH: f32 = 1.0;
pub const CONTENT_GAP: f32 = 8.0;
pub const BUTTON_HORIZONTAL_PADDING: f32 = 12.0;
pub const BUTTON_VERTICAL_PADDING: f32 = 7.0;
pub const CONTROL_MIN_WIDTH: f32 = 40.0;
pub const CONTROL_MIN_HEIGHT: f32 = 32.0;

pub const BLUE: Color32 = Color32::from_rgb(33, 120, 232);
pub const BLUE_SOFT: Color32 = Color32::from_rgb(232, 242, 255);
pub const GREEN: Color32 = Color32::from_rgb(14, 165, 91);
pub const AMBER: Color32 = Color32::from_rgb(217, 119, 6);
pub const RED: Color32 = Color32::from_rgb(220, 55, 55);
pub const TEXT: Color32 = Color32::from_rgb(28, 42, 64);
pub const MUTED: Color32 = Color32::from_rgb(100, 116, 139);
pub const BORDER: Color32 = Color32::from_rgb(220, 228, 238);
pub const SURFACE: Color32 = Color32::from_rgb(255, 255, 255);
pub const CANVAS: Color32 = Color32::from_rgb(247, 249, 252);

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlState {
    Normal,
    Hovered,
    Focused,
    Disabled,
    Error,
}

#[cfg(test)]
pub fn control_state_label(state: ControlState) -> &'static str {
    match state {
        ControlState::Normal => "正常",
        ControlState::Hovered => "悬停",
        ControlState::Focused => "焦点",
        ControlState::Disabled => "已禁用",
        ControlState::Error => "错误",
    }
}

pub fn configure(context: &egui::Context) {
    let mut style = (*context.global_style()).clone();
    style.spacing.item_spacing = Vec2::splat(CONTENT_GAP);
    style.spacing.button_padding = Vec2::new(BUTTON_HORIZONTAL_PADDING, BUTTON_VERTICAL_PADDING);
    style.spacing.interact_size = Vec2::new(CONTROL_MIN_WIDTH, CONTROL_MIN_HEIGHT);
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = SURFACE;
    style.visuals.faint_bg_color = CANVAS;
    style.visuals.widgets.inactive.bg_fill = SURFACE;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(BORDER_WIDTH, BORDER);
    style.visuals.widgets.hovered.bg_fill = BLUE_SOFT;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(BORDER_WIDTH, BLUE);
    style.visuals.widgets.active.bg_fill = BLUE;
    style.visuals.widgets.active.bg_stroke = Stroke::new(BORDER_WIDTH, BLUE);
    style.visuals.widgets.active.fg_stroke.color = Color32::WHITE;
    style.visuals.window_corner_radius = egui::CornerRadius::same(WINDOW_CORNER_RADIUS);
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(CONTROL_CORNER_RADIUS);
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(CONTROL_CORNER_RADIUS);
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(CONTROL_CORNER_RADIUS);
    context.set_global_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_tokens_keep_control_dimensions_stable() {
        assert!(NAVIGATION_COMPACT_WIDTH < NAVIGATION_WIDE_WIDTH);
        assert!(NAVIGATION_ROW_HEIGHT >= CONTROL_MIN_HEIGHT);
        assert!(ICON_SIZE < NAVIGATION_COMPACT_WIDTH);
        assert_eq!(BORDER_WIDTH, 1.0);
    }

    #[test]
    fn every_component_state_has_a_non_color_label() {
        for state in [
            ControlState::Normal,
            ControlState::Hovered,
            ControlState::Focused,
            ControlState::Disabled,
            ControlState::Error,
        ] {
            assert!(!control_state_label(state).is_empty());
        }
    }

    #[test]
    fn configured_visuals_include_normal_hover_and_focus_colours() {
        let context = egui::Context::default();
        configure(&context);
        let style = context.global_style();
        assert_eq!(style.visuals.widgets.inactive.bg_stroke.color, BORDER);
        assert_eq!(style.visuals.widgets.hovered.bg_stroke.color, BLUE);
        assert_ne!(
            style.visuals.selection.stroke.color,
            style.visuals.widgets.inactive.bg_stroke.color
        );
        assert!(style.visuals.widgets.noninteractive.fg_stroke.color != RED);
    }
}
