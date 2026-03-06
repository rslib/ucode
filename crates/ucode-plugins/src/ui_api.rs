use crate::manifest::PluginCapabilities;

/// Safety classification for a plugin UI call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiCallClass {
    /// Always allowed; no capability declaration required.
    Safe,
    /// Allowed only when the plugin declares `guarded_ui = true`.
    Guarded,
    /// Always blocked regardless of declared capabilities.
    Risky,
}

/// All UI extension calls a plugin may issue to the host.
#[derive(Debug, Clone)]
pub enum PluginUiCall {
    // --- Safe calls ---
    Toast {
        level: String,
        title: String,
        body: Option<String>,
        duration_ms: Option<u64>,
    },
    Notify {
        level: String,
        message: String,
    },
    SidebarSection {
        id: String,
        title: String,
        lines: Vec<String>,
        priority: Option<i32>,
    },
    StatusSegment {
        id: String,
        content: String,
        priority: Option<i32>,
    },
    PaletteCommand {
        name: String,
        description: String,
    },
    Badge {
        section_id: String,
        count: u32,
        level: String,
    },

    // --- Guarded calls ---
    TranscriptEvent {
        style: String,
        content: String,
    },
    Modal {
        title: String,
        content: String,
        actions: Vec<String>,
    },
    Confirm {
        title: String,
        message: String,
    },
    InputPrompt {
        title: String,
        placeholder: String,
    },
}

impl PluginUiCall {
    /// Safety classification for this call variant.
    pub fn call_class(&self) -> UiCallClass {
        match self {
            Self::Toast { .. }
            | Self::Notify { .. }
            | Self::SidebarSection { .. }
            | Self::StatusSegment { .. }
            | Self::PaletteCommand { .. }
            | Self::Badge { .. } => UiCallClass::Safe,

            Self::TranscriptEvent { .. }
            | Self::Modal { .. }
            | Self::Confirm { .. }
            | Self::InputPrompt { .. } => UiCallClass::Guarded,
        }
    }

    /// Canonical name used in error messages.
    fn call_name(&self) -> &'static str {
        match self {
            Self::Toast { .. } => "Toast",
            Self::Notify { .. } => "Notify",
            Self::SidebarSection { .. } => "SidebarSection",
            Self::StatusSegment { .. } => "StatusSegment",
            Self::PaletteCommand { .. } => "PaletteCommand",
            Self::Badge { .. } => "Badge",
            Self::TranscriptEvent { .. } => "TranscriptEvent",
            Self::Modal { .. } => "Modal",
            Self::Confirm { .. } => "Confirm",
            Self::InputPrompt { .. } => "InputPrompt",
        }
    }
}

/// Errors returned when a plugin UI call is denied.
#[derive(Debug, thiserror::Error)]
pub enum UiCallDenied {
    /// Plugin issued a call requiring a capability it did not declare.
    #[error(
        "UI call '{call_name}' requires capability '{required_capability}' which was not declared"
    )]
    NotDeclared {
        call_name: String,
        required_capability: String,
    },
    /// Call class is Risky and is always blocked.
    #[error("UI call '{call_name}' is blocked (Risky calls are never permitted)")]
    Blocked { call_name: String },
}

/// Check whether `call` is permitted given the plugin's declared `capabilities`.
///
/// - [`UiCallClass::Safe`]: always permitted.
/// - [`UiCallClass::Guarded`]: permitted only when `capabilities.guarded_ui` is `true`.
/// - [`UiCallClass::Risky`]: always denied with [`UiCallDenied::Blocked`].
pub fn check_ui_call(
    call: &PluginUiCall,
    capabilities: &PluginCapabilities,
) -> Result<(), UiCallDenied> {
    match call.call_class() {
        UiCallClass::Safe => Ok(()),
        UiCallClass::Guarded => {
            if capabilities.guarded_ui {
                Ok(())
            } else {
                Err(UiCallDenied::NotDeclared {
                    call_name: call.call_name().to_owned(),
                    required_capability: "guarded_ui".to_owned(),
                })
            }
        }
        UiCallClass::Risky => Err(UiCallDenied::Blocked {
            call_name: call.call_name().to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::PluginCapabilities;

    fn default_caps() -> PluginCapabilities {
        PluginCapabilities::default()
    }

    fn guarded_caps() -> PluginCapabilities {
        PluginCapabilities {
            guarded_ui: true,
            ..PluginCapabilities::default()
        }
    }

    // ---- call_class() for Safe variants ----

    #[test]
    fn test_toast_is_safe() {
        let call = PluginUiCall::Toast {
            level: "info".into(),
            title: "Hello".into(),
            body: None,
            duration_ms: None,
        };
        assert_eq!(call.call_class(), UiCallClass::Safe);
    }

    #[test]
    fn test_notify_is_safe() {
        let call = PluginUiCall::Notify {
            level: "warn".into(),
            message: "Watch out".into(),
        };
        assert_eq!(call.call_class(), UiCallClass::Safe);
    }

    #[test]
    fn test_sidebar_section_is_safe() {
        let call = PluginUiCall::SidebarSection {
            id: "s1".into(),
            title: "My Section".into(),
            lines: vec!["line 1".into()],
            priority: None,
        };
        assert_eq!(call.call_class(), UiCallClass::Safe);
    }

    #[test]
    fn test_status_segment_is_safe() {
        let call = PluginUiCall::StatusSegment {
            id: "seg1".into(),
            content: "OK".into(),
            priority: Some(10),
        };
        assert_eq!(call.call_class(), UiCallClass::Safe);
    }

    #[test]
    fn test_palette_command_is_safe() {
        let call = PluginUiCall::PaletteCommand {
            name: "run-tests".into(),
            description: "Run all tests".into(),
        };
        assert_eq!(call.call_class(), UiCallClass::Safe);
    }

    #[test]
    fn test_badge_is_safe() {
        let call = PluginUiCall::Badge {
            section_id: "errors".into(),
            count: 3,
            level: "error".into(),
        };
        assert_eq!(call.call_class(), UiCallClass::Safe);
    }

    // ---- call_class() for Guarded variants ----

    #[test]
    fn test_transcript_event_is_guarded() {
        let call = PluginUiCall::TranscriptEvent {
            style: "bold".into(),
            content: "Important".into(),
        };
        assert_eq!(call.call_class(), UiCallClass::Guarded);
    }

    #[test]
    fn test_modal_is_guarded() {
        let call = PluginUiCall::Modal {
            title: "Confirm".into(),
            content: "Are you sure?".into(),
            actions: vec!["Yes".into(), "No".into()],
        };
        assert_eq!(call.call_class(), UiCallClass::Guarded);
    }

    #[test]
    fn test_confirm_is_guarded() {
        let call = PluginUiCall::Confirm {
            title: "Delete".into(),
            message: "This cannot be undone.".into(),
        };
        assert_eq!(call.call_class(), UiCallClass::Guarded);
    }

    #[test]
    fn test_input_prompt_is_guarded() {
        let call = PluginUiCall::InputPrompt {
            title: "Enter name".into(),
            placeholder: "Name...".into(),
        };
        assert_eq!(call.call_class(), UiCallClass::Guarded);
    }

    // ---- check_ui_call: Safe calls always allowed ----

    #[test]
    fn test_safe_call_allowed_with_default_caps() {
        let call = PluginUiCall::Toast {
            level: "info".into(),
            title: "Hi".into(),
            body: None,
            duration_ms: None,
        };
        assert!(check_ui_call(&call, &default_caps()).is_ok());
    }

    #[test]
    fn test_safe_call_allowed_without_guarded_ui() {
        let call = PluginUiCall::Badge {
            section_id: "x".into(),
            count: 1,
            level: "info".into(),
        };
        let caps = PluginCapabilities {
            guarded_ui: false,
            ..PluginCapabilities::default()
        };
        assert!(check_ui_call(&call, &caps).is_ok());
    }

    // ---- check_ui_call: Guarded calls blocked without capability ----

    #[test]
    fn test_guarded_call_blocked_without_capability() {
        let call = PluginUiCall::Modal {
            title: "T".into(),
            content: "C".into(),
            actions: vec![],
        };
        let err = check_ui_call(&call, &default_caps()).unwrap_err();
        assert!(matches!(err, UiCallDenied::NotDeclared { .. }));
    }

    #[test]
    fn test_confirm_blocked_without_capability() {
        let call = PluginUiCall::Confirm {
            title: "T".into(),
            message: "M".into(),
        };
        let err = check_ui_call(&call, &default_caps()).unwrap_err();
        assert!(matches!(err, UiCallDenied::NotDeclared { .. }));
    }

    // ---- check_ui_call: Guarded calls allowed with capability ----

    #[test]
    fn test_guarded_call_allowed_with_capability() {
        let call = PluginUiCall::Modal {
            title: "T".into(),
            content: "C".into(),
            actions: vec![],
        };
        assert!(check_ui_call(&call, &guarded_caps()).is_ok());
    }

    #[test]
    fn test_input_prompt_allowed_with_capability() {
        let call = PluginUiCall::InputPrompt {
            title: "T".into(),
            placeholder: "P".into(),
        };
        assert!(check_ui_call(&call, &guarded_caps()).is_ok());
    }

    // ---- UiCallDenied display messages ----

    #[test]
    fn test_not_declared_display() {
        let err = UiCallDenied::NotDeclared {
            call_name: "Modal".into(),
            required_capability: "guarded_ui".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("Modal"), "expected call name in: {msg}");
        assert!(msg.contains("guarded_ui"), "expected capability in: {msg}");
    }

    #[test]
    fn test_blocked_display() {
        let err = UiCallDenied::Blocked {
            call_name: "DangerousCall".into(),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("DangerousCall"),
            "expected call name in: {msg}"
        );
    }
}
