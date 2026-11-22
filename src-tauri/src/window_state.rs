//! Main-window observations shared by navigation and inline approval cards.
//! No native calls here: tests can reproduce contradictory platform readings.

#[derive(Clone, Copy)]
pub(crate) struct WindowState {
    pub visible: Option<bool>,
    pub minimized: Option<bool>,
    pub focused: Option<bool>,
    pub in_tray: bool,
}

impl WindowState {
    pub fn ui_available(self) -> bool {
        self.visible == Some(true) || self.focused == Some(true)
    }

    pub fn card_available(self) -> bool {
        !self.in_tray
            || self.ui_available()
            || self.visible.unwrap_or(true)
            || self.minimized.unwrap_or(true)
    }

    pub fn reconciled_in_tray(self) -> bool {
        self.in_tray
            && !(self.focused == Some(true)
                || (self.visible == Some(true) && self.minimized == Some(false)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hidden() -> WindowState {
        WindowState {
            visible: Some(false),
            minimized: Some(false),
            focused: Some(false),
            in_tray: true,
        }
    }

    #[test]
    fn focused_main_window_overrides_stale_hidden_observation() {
        let state = WindowState {
            focused: Some(true),
            ..hidden()
        };
        assert!(
            state.ui_available(),
            "typing in main is proof the UI is open"
        );
        assert!(
            state.card_available(),
            "an open window must receive its card"
        );
        assert!(
            !state.reconciled_in_tray(),
            "focus repairs stale tray state"
        );
    }

    #[test]
    fn shown_unminimized_window_repairs_stale_tray_state() {
        let state = WindowState {
            visible: Some(true),
            ..hidden()
        };
        assert!(state.ui_available());
        assert!(state.card_available());
        assert!(!state.reconciled_in_tray());
    }

    #[test]
    fn genuinely_closed_to_tray_still_refuses_ui_and_cards() {
        assert!(!hidden().ui_available());
        assert!(!hidden().card_available());
        assert!(hidden().reconciled_in_tray());
    }

    #[test]
    fn minimized_or_hidden_by_macos_keeps_card_wait_without_claiming_visible_ui() {
        let minimized = WindowState {
            minimized: Some(true),
            ..hidden()
        };
        assert!(minimized.card_available());
        assert!(!minimized.ui_available());
        assert!(minimized.reconciled_in_tray());
        let app_hidden = WindowState {
            in_tray: false,
            ..hidden()
        };
        assert!(app_hidden.card_available());
        assert!(!app_hidden.ui_available());
    }

    #[test]
    fn unreadable_visibility_with_focus_is_open_but_unknown_state_stays_unknown() {
        let focused = WindowState {
            visible: None,
            focused: Some(true),
            ..hidden()
        };
        assert!(focused.ui_available());
        assert!(!focused.reconciled_in_tray());
        let unknown = WindowState {
            visible: None,
            minimized: None,
            focused: None,
            ..hidden()
        };
        assert!(!unknown.ui_available());
        assert!(
            unknown.card_available(),
            "preserve the existing card wait on unknown state"
        );
        assert!(unknown.reconciled_in_tray());
    }
}
