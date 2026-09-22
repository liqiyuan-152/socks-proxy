#[cfg(windows)]
use crate::{core::SwitchPhase, routing::RoutingMode, ui::UiState};

#[cfg(windows)]
pub fn import_allowed(state: &UiState) -> bool {
    state.runtime.phase == SwitchPhase::Direct
        && state.runtime.applied_mode == Some(RoutingMode::Direct)
}
