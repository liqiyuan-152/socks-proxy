use crate::{core::SwitchPhase, routing::RoutingMode, ui::UiState};

pub fn mode_label(mode: RoutingMode) -> &'static str {
    match mode {
        RoutingMode::Direct => "全局直连",
        RoutingMode::Rules => "规则代理",
        RoutingMode::GlobalProxy => "全局代理",
    }
}

pub fn is_applying(state: &UiState) -> bool {
    state.runtime.phase == SwitchPhase::Reconfiguring
}

pub fn global_phase_label(phase: SwitchPhase) -> &'static str {
    match phase {
        SwitchPhase::Direct => "内核未运行",
        SwitchPhase::Running => "内核运行中",
        SwitchPhase::Reconfiguring => "正在应用",
        SwitchPhase::Error => "运行异常",
    }
}
