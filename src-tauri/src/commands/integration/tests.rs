#[cfg(test)]
mod integration_disconnect_tests {
    use super::super::{
        auto_connect_action, integration_for_notify_source, is_integration_disconnected,
        managed_status_from_cc, managed_status_from_hook_check, notify_source_for_integration,
        session_ids_for_integration, AutoConnectAction, CcIntegrationStatus,
    };
    use crate::sessions::{Session, SessionStatus};
    use crate::settings::SettingsStore;
    use chrono::Utc;
    use std::sync::{Arc, Mutex};

    fn sample_session(id: &str, source: &str) -> Session {
        let now = Utc::now();
        Session {
            id: id.into(),
            task_id: format!("task-{id}"),
            external_session_id: None,
            title: "t".into(),
            message: "m".into(),
            source: Some(source.into()),
            priority: Default::default(),
            status: SessionStatus::Running,
            created_at: now,
            updated_at: now,
            terminal_tty: None,
            workspace_path: None,
            failure_reason: None,
            aborted_at: None,
            terminal_id: None,
            terminal_id_captured_cwd: None,
            transcript_path: None,
            herdr_pane_id: None,
            herdr_workspace_id: None,
            herdr_socket_path: None,
            cmux_surface_id: None,
            cmux_workspace_id: None,
            wezterm_pane_id: None,
            wezterm_unix_socket: None,
            wezterm_executable: None,
            otty_pane_id: None,
            otty_tab_id: None,
            otty_window_id: None,
            tmux_pane_id: None,
            tmux_socket_path: None,
            orca_pane_key: None,
            orca_worktree_id: None,
            orca_tab_id: None,
            terminal_app: None,
            dsh_nav_socket: None,
            surface: Default::default(), // W5:内层绑定真源,见 brief 20260813-surface-binding
        }
    }

    #[test]
    fn notify_source_mapping_roundtrip() {
        assert_eq!(notify_source_for_integration("cc"), Some("claude-code"));
        assert_eq!(integration_for_notify_source("claude-code"), Some("cc"));
        assert_eq!(integration_for_notify_source("cursor"), Some("cursor"));
    }

    #[test]
    fn disconnected_integration_is_detected_in_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Mutex::new(SettingsStore::load(
            dir.path().join("settings.json"),
        )));
        {
            let mut g = store.lock().unwrap_or_else(|p| p.into_inner());
            g.settings.disconnected_sources.push("cursor".into());
            g.save();
        }
        assert!(is_integration_disconnected(&store, "cursor"));
        assert!(!is_integration_disconnected(&store, "cc"));
    }

    #[test]
    fn session_ids_for_integration_filters_by_notify_source() {
        let sessions = vec![
            sample_session("a", "claude-code"),
            sample_session("b", "cursor"),
            sample_session("c", "codex"),
        ];
        assert_eq!(
            session_ids_for_integration(&sessions, "cc"),
            vec!["a".to_string()]
        );
        assert_eq!(
            session_ids_for_integration(&sessions, "cursor"),
            vec!["b".to_string()]
        );
        assert!(session_ids_for_integration(&sessions, "warp").is_empty());
    }

    #[test]
    fn managed_status_from_cc_hides_issue_when_disconnected() {
        let cc = CcIntegrationStatus {
            installed: true,
            binary_executable: true,
            settings_exists: false,
            hooks_configured: false,
            connected: false,
            repair_available: true,
            issue: "hooks_missing".into(),
        };
        let row = managed_status_from_cc(&cc, true, false, true);
        assert!(row.disconnected);
        assert!(!row.connected);
        assert!(row.issue.is_none());
    }

    #[test]
    fn managed_status_from_cc_connected_unless_disconnected() {
        let cc = CcIntegrationStatus {
            installed: true,
            binary_executable: true,
            settings_exists: true,
            hooks_configured: true,
            connected: true,
            repair_available: true,
            issue: "healthy".into(),
        };
        let connected = managed_status_from_cc(&cc, true, false, false);
        assert!(connected.connected);
        assert_eq!(connected.issue.as_deref(), Some("healthy"));

        let disconnected = managed_status_from_cc(&cc, true, false, true);
        assert!(!disconnected.connected);
        assert!(disconnected.disconnected);
    }

    #[test]
    fn managed_status_from_hook_check_applies_disconnected_gate() {
        let check = serde_json::json!({
            "connected": true,
            "hooks_configured": true,
            "installed": true,
        });
        let row = managed_status_from_hook_check("cursor", true, &check, false, true, true);
        assert!(!row.connected);
        assert!(row.disconnected);
        assert!(row.hooks_configured);
    }

    #[test]
    fn auto_connect_action_matrix() {
        assert_eq!(
            auto_connect_action(false, false, false, false),
            AutoConnectAction::SkipAgentNotPresent
        );
        assert_eq!(
            auto_connect_action(true, true, false, false),
            AutoConnectAction::RecordConnected
        );
        assert_eq!(
            auto_connect_action(true, false, true, true),
            AutoConnectAction::InstallOutdated
        );
        assert_eq!(
            auto_connect_action(true, false, false, true),
            AutoConnectAction::SkipUserDisconnected
        );
        assert_eq!(
            auto_connect_action(true, false, false, false),
            AutoConnectAction::InstallNew
        );
    }
}

#[cfg(test)]
mod hook_drift_state_tests {
    use super::super::HookDriftState;

    #[test]
    fn set_outdated_replaces_previous_set() {
        let mut s = HookDriftState::default();
        s.set_outdated(&["cc".into()]);
        assert_eq!(s.outdated_sources(), vec!["cc".to_string()]);
        s.set_outdated(&["cursor".into()]);
        assert_eq!(s.outdated_sources(), vec!["cursor".to_string()]);
    }

    #[test]
    fn outdated_sources_returns_sorted() {
        let mut s = HookDriftState::default();
        s.set_outdated(&["cursor".into(), "cc".into()]);
        assert_eq!(
            s.outdated_sources(),
            vec!["cc".to_string(), "cursor".to_string()]
        );
    }

    #[test]
    fn clear_source_removes_only_target() {
        let mut s = HookDriftState::default();
        s.set_outdated(&["cc".into(), "cursor".into()]);
        s.clear_source("cc");
        assert_eq!(s.outdated_sources(), vec!["cursor".to_string()]);
    }

    #[test]
    fn clear_source_unknown_is_noop() {
        let mut s = HookDriftState::default();
        s.set_outdated(&["cc".into()]);
        s.clear_source("nonexistent");
        assert_eq!(s.outdated_sources(), vec!["cc".to_string()]);
    }

    #[test]
    fn empty_state_returns_empty_vec() {
        let s = HookDriftState::default();
        assert!(s.outdated_sources().is_empty());
    }
}
