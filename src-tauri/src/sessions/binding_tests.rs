//! sessions 单测:SurfaceBinding 存储层 ops(W5,brief 20260813-surface-binding)。
//! 从 `tests.rs` 拆出(编排者批准,先例 tests_license / pokehook 各测试文件),
//! 共享 helper 经 `super::tests` 引用。

use super::tests::{insert_running_session, insert_running_session_with_id, temp_store};
use super::SessionStore;
use crate::surface::SurfaceBinding;

// ==================== surface binding ops(W5,brief 20260813-surface-binding) ====================
// 原 set/clear_herdr/cmux/tmux/orca/wezterm/otty_binding 测试改写为对新 op
// 的等价断言:surface 为真源,平铺字段为 legacy mirror(双写断言)。

#[test]
fn set_surface_binding_herdr_writes_surface_and_mirror() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let changed = store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: Some("w3".into()),
            socket_path: Some("/tmp/herdr.sock".into()),
        },
    );
    assert!(changed);
    let s = &store.sessions[0];
    // surface 真源
    assert_eq!(
        s.surface,
        SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: Some("w3".into()),
            socket_path: Some("/tmp/herdr.sock".into()),
        }
    );
    // 平铺镜像同步
    assert_eq!(s.herdr_pane_id.as_deref(), Some("w3:p1"));
    assert_eq!(s.herdr_workspace_id.as_deref(), Some("w3"));
    assert_eq!(s.herdr_socket_path.as_deref(), Some("/tmp/herdr.sock"));
}

#[test]
fn set_surface_binding_herdr_is_idempotent_on_same_values() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let b = SurfaceBinding::Herdr {
        pane_id: "w3:p1".into(),
        workspace_id: Some("w3".into()),
        socket_path: Some("/s".into()),
    };
    store.set_surface_binding("cc-123", b.clone());
    let changed = store.set_surface_binding("cc-123", b);
    assert!(!changed, "同值应幂等返回 false");
}

#[test]
fn set_surface_binding_herdr_overwrites_on_different_pane_id() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: Some("w3".into()),
            socket_path: Some("/s".into()),
        },
    );
    let changed = store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Herdr {
            pane_id: "w3:p2".into(),
            workspace_id: Some("w3".into()),
            socket_path: Some("/s".into()),
        },
    );
    assert!(
        changed,
        "pane_id 变化必须覆盖(herdr split/kill 会换 pane_id)"
    );
    assert_eq!(store.sessions[0].herdr_pane_id.as_deref(), Some("w3:p2"));
}

#[test]
fn set_surface_binding_herdr_clears_cmux_mirror() {
    // 互斥复刻:写 herdr 清 cmux(原 set_herdr_binding 行为)。
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Cmux {
            surface_id: "SURF-A".into(),
            workspace_id: Some("WS-A".into()),
        },
    );
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: Some("w3".into()),
            socket_path: Some("/s".into()),
        },
    );
    let s = &store.sessions[0];
    assert_eq!(
        s.surface,
        SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: Some("w3".into()),
            socket_path: Some("/s".into()),
        }
    );
    assert!(s.cmux_surface_id.is_none(), "herdr 写后 cmux 镜像应被清");
    assert!(s.cmux_workspace_id.is_none());
}

#[test]
fn set_surface_binding_missing_task_returns_false() {
    let (_dir, mut store, _path) = temp_store();
    assert!(!store.set_surface_binding(
        "nope",
        SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: Some("w3".into()),
            socket_path: Some("/s".into()),
        },
    ));
}

#[test]
fn clear_surface_binding_herdr_matches_and_clears() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let sid = store.sessions[0].id.clone();
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: Some("w3".into()),
            socket_path: Some("/s".into()),
        },
    );
    // 调用点只持有主键 pane_id,workspace/socket 传 None —— 主键匹配仍命中。
    assert!(store.clear_surface_binding_if_matches(
        &sid,
        &SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: None,
            socket_path: None,
        },
    ));
    let s = &store.sessions[0];
    assert_eq!(s.surface, SurfaceBinding::None);
    assert!(s.herdr_pane_id.is_none());
    assert!(s.herdr_workspace_id.is_none());
    assert!(s.herdr_socket_path.is_none());
}

#[test]
fn clear_surface_binding_herdr_no_op_on_mismatch() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let sid = store.sessions[0].id.clone();
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: Some("w3".into()),
            socket_path: Some("/s".into()),
        },
    );
    assert!(!store.clear_surface_binding_if_matches(
        &sid,
        &SurfaceBinding::Herdr {
            pane_id: "w3:p999".into(),
            workspace_id: None,
            socket_path: None,
        },
    ));
    assert_eq!(store.sessions[0].herdr_pane_id.as_deref(), Some("w3:p1"));
}

#[test]
fn set_surface_binding_cmux_writes_surface_and_mirror() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let changed = store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Cmux {
            surface_id: "FABEB780-8D56-4548-8D92-1E4CB60B7E68".into(),
            workspace_id: Some("4C78AC55-7559-49CF-8C75-784F722C1491".into()),
        },
    );
    assert!(changed);
    let s = &store.sessions[0];
    assert_eq!(
        s.surface,
        SurfaceBinding::Cmux {
            surface_id: "FABEB780-8D56-4548-8D92-1E4CB60B7E68".into(),
            workspace_id: Some("4C78AC55-7559-49CF-8C75-784F722C1491".into()),
        }
    );
    assert_eq!(
        s.cmux_surface_id.as_deref(),
        Some("FABEB780-8D56-4548-8D92-1E4CB60B7E68")
    );
    assert_eq!(
        s.cmux_workspace_id.as_deref(),
        Some("4C78AC55-7559-49CF-8C75-784F722C1491")
    );
}

#[test]
fn set_surface_binding_cmux_is_idempotent_on_same_values() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let b = SurfaceBinding::Cmux {
        surface_id: "SURF-A".into(),
        workspace_id: Some("WS-A".into()),
    };
    store.set_surface_binding("cc-123", b.clone());
    let changed = store.set_surface_binding("cc-123", b);
    assert!(!changed, "同 surface_id + workspace_id 应幂等返回 false");
}

#[test]
fn set_surface_binding_cmux_overwrites_on_different_surface_id() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Cmux {
            surface_id: "SURF-A".into(),
            workspace_id: Some("WS-A".into()),
        },
    );
    let changed = store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Cmux {
            surface_id: "SURF-B".into(),
            workspace_id: Some("WS-A".into()),
        },
    );
    assert!(changed, "surface_id 变化必须覆盖(cmux pane 重建会换 UUID)");
    assert_eq!(store.sessions[0].cmux_surface_id.as_deref(), Some("SURF-B"));
}

#[test]
fn clear_surface_binding_cmux_matches_and_clears() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let sid = store.sessions[0].id.clone();
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Cmux {
            surface_id: "SURF-A".into(),
            workspace_id: Some("WS-A".into()),
        },
    );
    assert!(store.clear_surface_binding_if_matches(
        &sid,
        &SurfaceBinding::Cmux {
            surface_id: "SURF-A".into(),
            workspace_id: None,
        },
    ));
    let s = &store.sessions[0];
    assert_eq!(s.surface, SurfaceBinding::None);
    assert!(s.cmux_surface_id.is_none());
    assert!(s.cmux_workspace_id.is_none());
}

#[test]
fn clear_surface_binding_cmux_no_op_on_mismatch() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let sid = store.sessions[0].id.clone();
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Cmux {
            surface_id: "SURF-A".into(),
            workspace_id: Some("WS-A".into()),
        },
    );
    // race:reconcile 换了 surface_id,caller 拿的老值不应清
    assert!(!store.clear_surface_binding_if_matches(
        &sid,
        &SurfaceBinding::Cmux {
            surface_id: "SURF-OLD".into(),
            workspace_id: None,
        },
    ));
    assert_eq!(store.sessions[0].cmux_surface_id.as_deref(), Some("SURF-A"));
}

#[test]
fn set_surface_binding_tmux_writes_surface_and_mirror() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let changed = store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Tmux {
            pane_id: "%12".into(),
            socket_path: Some("/tmp/tmux-501/default".into()),
        },
    );
    assert!(changed);
    let s = &store.sessions[0];
    assert_eq!(
        s.surface,
        SurfaceBinding::Tmux {
            pane_id: "%12".into(),
            socket_path: Some("/tmp/tmux-501/default".into()),
        }
    );
    assert_eq!(s.tmux_pane_id.as_deref(), Some("%12"));
    assert_eq!(s.tmux_socket_path.as_deref(), Some("/tmp/tmux-501/default"));
}

#[test]
fn set_surface_binding_tmux_is_idempotent_on_same_values() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let b = SurfaceBinding::Tmux {
        pane_id: "%12".into(),
        socket_path: Some("/s".into()),
    };
    store.set_surface_binding("cc-123", b.clone());
    let changed = store.set_surface_binding("cc-123", b);
    assert!(!changed, "同 pane_id + socket 应幂等返回 false");
}

#[test]
fn set_surface_binding_tmux_overwrites_on_different_pane_id() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Tmux {
            pane_id: "%12".into(),
            socket_path: Some("/s".into()),
        },
    );
    let changed = store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Tmux {
            pane_id: "%34".into(),
            socket_path: Some("/s".into()),
        },
    );
    assert!(changed, "pane_id 变化必须覆盖(tmux pane 重建会换 id)");
    assert_eq!(store.sessions[0].tmux_pane_id.as_deref(), Some("%34"));
}

#[test]
fn set_surface_binding_orca_clears_otty_mirror_and_guards_empty_worktree() {
    // 互斥复刻:写 orca 清 otty(原 set_orca_binding 行为,Orca 宿主优先)。
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Otty {
            pane_id: "p_1".into(),
            tab_id: Some("t_1".into()),
            window_id: None,
        },
    );
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Orca {
            pane_key: "tab1:leaf1".into(),
            worktree_id: Some("wt-1".into()),
            tab_id: None,
        },
    );
    let s = &store.sessions[0];
    assert_eq!(
        s.surface,
        SurfaceBinding::Orca {
            pane_key: "tab1:leaf1".into(),
            worktree_id: Some("wt-1".into()),
            tab_id: None,
        }
    );
    assert!(s.otty_pane_id.is_none(), "orca 写后 otty 镜像应被清");
    assert!(s.otty_tab_id.is_none());
    assert!(s.otty_window_id.is_none());
    // worktree_id 空 → 拒写(原 set_orca_binding 守卫,防幂等被空串打破)。
    let refused = store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Orca {
            pane_key: "tab1:leaf2".into(),
            worktree_id: Some(String::new()),
            tab_id: None,
        },
    );
    assert!(!refused, "空 worktree_id 应拒写");
    assert_eq!(
        store.sessions[0].orca_pane_key.as_deref(),
        Some("tab1:leaf1"),
        "拒写后旧 binding 不变"
    );
}

#[test]
fn set_surface_binding_herdr_and_orca_coexist_in_mirror() {
    // 共存语义(编排者修订裁决):herdr 跑在 Orca 外层时同 session 同时持有
    // herdr + orca(见 commands/focus/orca_herdr.rs::apply_orca_binding_for_herdr_host_if_missing)。
    // 单值 surface 只承载其一,但**平铺镜像必须共存** —— 写 orca 不清 herdr 镜像。
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: Some("w3".into()),
            socket_path: Some("/s".into()),
        },
    );
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Orca {
            pane_key: "tab1:leaf1".into(),
            worktree_id: Some("wt-1".into()),
            tab_id: None,
        },
    );
    let s = &store.sessions[0];
    assert_eq!(
        s.herdr_pane_id.as_deref(),
        Some("w3:p1"),
        "orca 写不应清 herdr 镜像"
    );
    assert_eq!(s.herdr_socket_path.as_deref(), Some("/s"));
    assert_eq!(s.orca_pane_key.as_deref(), Some("tab1:leaf1"));
}

#[test]
fn set_surface_binding_if_missing_skips_when_bound() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.set_surface_binding(
        "cc-123",
        SurfaceBinding::Tmux {
            pane_id: "%12".into(),
            socket_path: Some("/s".into()),
        },
    );
    // 已有绑定 → 不覆盖
    let changed = store.set_surface_binding_if_missing(
        "cc-123",
        SurfaceBinding::Cmux {
            surface_id: "SURF-A".into(),
            workspace_id: None,
        },
    );
    assert!(!changed, "已有绑定时不覆盖");
    assert_eq!(store.sessions[0].tmux_pane_id.as_deref(), Some("%12"));
    // 无绑定 → 写入
    insert_running_session_with_id(&mut store, "cc-456");
    let changed = store.set_surface_binding_if_missing(
        "cc-456",
        SurfaceBinding::Cmux {
            surface_id: "SURF-A".into(),
            workspace_id: None,
        },
    );
    assert!(changed);
    let s = store
        .sessions
        .iter()
        .find(|s| s.task_id == "cc-456")
        .unwrap();
    assert_eq!(s.cmux_surface_id.as_deref(), Some("SURF-A"));
}

#[test]
fn load_migrates_flat_bindings_to_surface() {
    // W5 加载迁移:老 sessions.json(无 surface 字段)→ load 后 surface 从平铺派生。
    let (_dir, store, path) = temp_store();
    drop(store);
    let now = chrono::Utc::now();
    let old_json = serde_json::json!([{
        "id": "s-1",
        "task_id": "cc-1",
        "title": "t",
        "message": "m",
        "source": "claude-code",
        "priority": "normal",
        "status": "idle",
        "created_at": now,
        "updated_at": now,
        "terminal_tty": "/dev/ttys001",
        "herdr_pane_id": "w3:p1",
        "herdr_workspace_id": "w3",
        "herdr_socket_path": "/tmp/herdr.sock"
    }]);
    std::fs::write(&path, serde_json::to_string(&old_json).unwrap()).unwrap();
    let store = SessionStore::load(path);
    let s = &store.sessions[0];
    assert_eq!(
        s.surface,
        SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: Some("w3".into()),
            socket_path: Some("/tmp/herdr.sock".into()),
        },
        "load 后 surface 应从平铺派生"
    );
    // 平铺镜像保留(读侧 W6 前仍用)
    assert_eq!(s.herdr_pane_id.as_deref(), Some("w3:p1"));
}

#[test]
fn load_keeps_existing_surface_untouched() {
    // 新文件已带 surface:迁移不得覆盖。
    let (_dir, store, path) = temp_store();
    drop(store);
    let now = chrono::Utc::now();
    let json = serde_json::json!([{
        "id": "s-1",
        "task_id": "cc-1",
        "title": "t",
        "message": "m",
        "source": "claude-code",
        "priority": "normal",
        "status": "idle",
        "created_at": now,
        "updated_at": now,
        "surface": {"kind": "tmux", "pane_id": "%9", "socket_path": "/s"}
    }]);
    std::fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();
    let store = SessionStore::load(path);
    assert_eq!(
        store.sessions[0].surface,
        SurfaceBinding::Tmux {
            pane_id: "%9".into(),
            socket_path: Some("/s".into()),
        }
    );
}
