//! surface 单测:SurfaceBinding(W5,brief 20260813-surface-binding)。
//! 从 `surface.rs` 拆出(编排者批准),经 `super::*` 引用 surface 模块。

use super::SurfaceBinding;
use crate::sessions::{Priority, Session, SessionStatus};
use chrono::Utc;

fn session_with(fields: &[(&str, Option<String>)]) -> Session {
    let now = Utc::now();
    let mut s = Session {
        id: "s1".into(),
        task_id: "cc-1".into(),
        external_session_id: None,
        title: "t".into(),
        message: "m".into(),
        source: Some("claude-code".into()),
        priority: Priority::Normal,
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
        surface: SurfaceBinding::None,
    };
    for (field, value) in fields {
        match *field {
            "herdr_pane_id" => s.herdr_pane_id = value.clone(),
            "herdr_workspace_id" => s.herdr_workspace_id = value.clone(),
            "herdr_socket_path" => s.herdr_socket_path = value.clone(),
            "cmux_surface_id" => s.cmux_surface_id = value.clone(),
            "cmux_workspace_id" => s.cmux_workspace_id = value.clone(),
            "wezterm_pane_id" => s.wezterm_pane_id = value.clone(),
            "wezterm_unix_socket" => s.wezterm_unix_socket = value.clone(),
            "wezterm_executable" => s.wezterm_executable = value.clone(),
            "otty_pane_id" => s.otty_pane_id = value.clone(),
            "otty_tab_id" => s.otty_tab_id = value.clone(),
            "otty_window_id" => s.otty_window_id = value.clone(),
            "tmux_pane_id" => s.tmux_pane_id = value.clone(),
            "tmux_socket_path" => s.tmux_socket_path = value.clone(),
            "orca_pane_key" => s.orca_pane_key = value.clone(),
            "orca_worktree_id" => s.orca_worktree_id = value.clone(),
            "orca_tab_id" => s.orca_tab_id = value.clone(),
            _ => panic!("unknown field {field}"),
        }
    }
    s
}

fn v(s: &str) -> Option<String> {
    Some(s.to_string())
}

#[test]
fn surface_binding_serde_round_trip_with_tag_shape() {
    // tagged enum 形状:{"kind": "herdr", "pane_id": ..., ...}。
    let b = SurfaceBinding::Herdr {
        pane_id: "w3:p1".into(),
        workspace_id: Some("w3".into()),
        socket_path: Some("/s".into()),
    };
    let json = serde_json::to_value(&b).unwrap();
    assert_eq!(json["kind"], "herdr");
    assert_eq!(json["pane_id"], "w3:p1");
    assert_eq!(json["workspace_id"], "w3");
    assert_eq!(json["socket_path"], "/s");
    let back: SurfaceBinding = serde_json::from_value(json).unwrap();
    assert_eq!(back, b);

    let none = SurfaceBinding::None;
    let json = serde_json::to_value(&none).unwrap();
    assert_eq!(json["kind"], "none");
    let back: SurfaceBinding = serde_json::from_value(json).unwrap();
    assert_eq!(back, SurfaceBinding::None);
}

#[test]
fn from_session_priority_follows_focus_ladder() {
    // focus ladder(commands/focus/short_circuit.rs::run_terminal_short_circuits):
    // herdr → cmux → wezterm → orca → otty → tmux。多绑定并存时取最高优先。
    type Case = (Vec<(&'static str, Option<String>)>, SurfaceBinding);
    let cases: Vec<Case> = vec![
        (
            vec![("herdr_pane_id", v("w3:p1")), ("tmux_pane_id", v("%9"))],
            SurfaceBinding::Herdr {
                pane_id: "w3:p1".into(),
                workspace_id: None,
                socket_path: None,
            },
        ),
        (
            vec![
                ("cmux_surface_id", v("SURF-A")),
                ("wezterm_pane_id", v("wp1")),
                ("otty_pane_id", v("p_1")),
            ],
            SurfaceBinding::Cmux {
                surface_id: "SURF-A".into(),
                workspace_id: None,
            },
        ),
        (
            vec![("wezterm_pane_id", v("wp1")), ("tmux_pane_id", v("%9"))],
            SurfaceBinding::Wezterm {
                pane_id: "wp1".into(),
                unix_socket: None,
                executable: None,
            },
        ),
        (
            vec![
                ("orca_pane_key", v("tab1:leaf1")),
                ("otty_pane_id", v("p_1")),
            ],
            SurfaceBinding::Orca {
                pane_key: "tab1:leaf1".into(),
                worktree_id: None,
                tab_id: None,
            },
        ),
        (
            vec![("otty_pane_id", v("p_1")), ("tmux_pane_id", v("%9"))],
            SurfaceBinding::Otty {
                pane_id: "p_1".into(),
                tab_id: None,
                window_id: None,
            },
        ),
        (
            vec![("tmux_pane_id", v("%9")), ("tmux_socket_path", v("/s"))],
            SurfaceBinding::Tmux {
                pane_id: "%9".into(),
                socket_path: Some("/s".into()),
            },
        ),
        (vec![], SurfaceBinding::None),
    ];
    for (fields, expected) in cases {
        let s = session_with(&fields);
        assert_eq!(
            SurfaceBinding::from_session(&s),
            expected,
            "fields={fields:?}"
        );
    }
}

#[test]
fn from_session_carries_optional_fields() {
    let s = session_with(&[
        ("herdr_pane_id", v("w3:p1")),
        ("herdr_workspace_id", v("w3")),
        ("herdr_socket_path", v("/s")),
    ]);
    assert_eq!(
        SurfaceBinding::from_session(&s),
        SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: Some("w3".into()),
            socket_path: Some("/s".into()),
        }
    );
}

#[test]
fn primary_key_per_variant() {
    assert_eq!(
        SurfaceBinding::Herdr {
            pane_id: "p1".into(),
            workspace_id: None,
            socket_path: None,
        }
        .primary_key(),
        Some("p1")
    );
    assert_eq!(
        SurfaceBinding::Cmux {
            surface_id: "S".into(),
            workspace_id: None,
        }
        .primary_key(),
        Some("S")
    );
    assert_eq!(
        SurfaceBinding::Orca {
            pane_key: "k".into(),
            worktree_id: None,
            tab_id: None,
        }
        .primary_key(),
        Some("k")
    );
    assert_eq!(SurfaceBinding::None.primary_key(), None);
    assert!(SurfaceBinding::Herdr {
        pane_id: "p1".into(),
        workspace_id: None,
        socket_path: None,
    }
    .same_variant(&SurfaceBinding::Herdr {
        pane_id: "p2".into(),
        workspace_id: None,
        socket_path: None,
    }));
    assert!(!SurfaceBinding::Herdr {
        pane_id: "p1".into(),
        workspace_id: None,
        socket_path: None,
    }
    .same_variant(&SurfaceBinding::Tmux {
        pane_id: "p1".into(),
        socket_path: None,
    }));
}

// ==================== SurfaceEvidence 读侧(W6,brief 20260813-surface-readers) ====================

#[test]
fn evidence_reads_inner_bindings_from_surface() {
    let mut s = session_with(&[("wezterm_pane_id", v("wp1"))]);
    s.surface = SurfaceBinding::Wezterm {
        pane_id: "wp1".into(),
        unix_socket: Some("/tmp/wez.sock".into()),
        executable: None,
    };
    let ev = super::SurfaceEvidence::from_session(&s);
    assert_eq!(ev.wezterm_pane_id.as_deref(), Some("wp1"));
    assert_eq!(ev.wezterm_unix_socket.as_deref(), Some("/tmp/wez.sock"));
    assert!(ev.herdr_pane_id.is_none());
    assert!(ev.cmux_surface_id.is_none());
    assert!(ev.otty_pane_id.is_none());
    assert!(ev.tmux_pane_id.is_none());
    assert!(ev.orca_pane_key.is_none());
}

#[test]
fn evidence_ignores_stale_mirror_when_surface_none() {
    // W6 消除 stale 消费:surface=None 时,即便镜像残留旧绑定字段,evidence 也不读。
    let mut s = session_with(&[("cmux_surface_id", v("SURF-A"))]);
    s.surface = SurfaceBinding::None;
    let ev = super::SurfaceEvidence::from_session(&s);
    assert!(
        ev.cmux_surface_id.is_none(),
        "surface=None 时镜像 stale 不消费"
    );
}

#[test]
fn evidence_herdr_plus_orca_special_case_both_directions() {
    // 特例(对称双向):surface=Herdr 时 orca 从镜像读;surface=Orca 时 herdr
    // 从镜像读(两个写序都可达:notify 先 herdr 后 orca host hint / herdr 刷新)。
    // surface=Herdr(herdr 刷新后的写序)
    let mut s = session_with(&[
        ("herdr_pane_id", v("w3:p1")),
        ("herdr_socket_path", v("/s")),
        ("orca_pane_key", v("tab1:leaf1")),
        ("orca_worktree_id", v("wt-1")),
    ]);
    s.surface = SurfaceBinding::Herdr {
        pane_id: "w3:p1".into(),
        workspace_id: None,
        socket_path: Some("/s".into()),
    };
    let ev = super::SurfaceEvidence::from_session(&s);
    assert_eq!(ev.herdr_pane_id.as_deref(), Some("w3:p1"));
    assert_eq!(
        ev.orca_pane_key.as_deref(),
        Some("tab1:leaf1"),
        "surface=Herdr 时 orca 从镜像读"
    );
    assert_eq!(ev.orca_worktree_id.as_deref(), Some("wt-1"));

    // surface=Orca(notify 先 herdr 后 orca host hint 的写序)
    let mut s2 = session_with(&[
        ("herdr_pane_id", v("w3:p1")),
        ("herdr_socket_path", v("/s")),
        ("orca_pane_key", v("tab1:leaf1")),
    ]);
    s2.surface = SurfaceBinding::Orca {
        pane_key: "tab1:leaf1".into(),
        worktree_id: Some("wt-1".into()),
        tab_id: None,
    };
    let ev2 = super::SurfaceEvidence::from_session(&s2);
    assert_eq!(ev2.orca_pane_key.as_deref(), Some("tab1:leaf1"));
    assert_eq!(
        ev2.herdr_pane_id.as_deref(),
        Some("w3:p1"),
        "surface=Orca 时 herdr 从镜像读"
    );
    assert_eq!(ev2.herdr_socket_path.as_deref(), Some("/s"));
}

#[test]
fn orca_fields_returns_mirror_when_surface_herdr() {
    // W6 review fix:orca 字段读点(herdr 外层锚点)对称特例 ——
    // surface=Herdr 时 orca_fields 必须从镜像回退(herdr 刷新后的写序)。
    let mut s = session_with(&[
        ("herdr_pane_id", v("w3:p1")),
        ("orca_pane_key", v("tab1:leaf1")),
        ("orca_worktree_id", v("wt-1")),
    ]);
    s.surface = SurfaceBinding::Herdr {
        pane_id: "w3:p1".into(),
        workspace_id: None,
        socket_path: Some("/s".into()),
    };
    let orca = SurfaceBinding::orca_fields(&s);
    assert_eq!(
        orca,
        Some(("tab1:leaf1", Some("wt-1"), None)),
        "surface=Herdr 时 orca 从镜像回退"
    );
    // surface=Orca 时取 surface 本身
    let mut s2 = session_with(&[("orca_pane_key", v("tab1:leaf1"))]);
    s2.surface = SurfaceBinding::Orca {
        pane_key: "tab1:leaf1".into(),
        worktree_id: None,
        tab_id: None,
    };
    assert_eq!(
        SurfaceBinding::orca_fields(&s2),
        Some(("tab1:leaf1", None, None))
    );
    // 无 orca(纯 herdr,无镜像)→ None
    let s3 = session_with(&[("herdr_pane_id", v("w3:p1"))]);
    assert_eq!(SurfaceBinding::orca_fields(&s3), None);
}

#[test]
fn is_herdr_remote_source_single_point() {
    // W7(brief 20260813-focus-dispatch-table):herdr-remote 源判定单点。
    assert!(super::is_herdr_remote_source(Some("herdr-remote")));
    assert!(!super::is_herdr_remote_source(Some("claude-code")));
    assert!(!super::is_herdr_remote_source(None));
}
