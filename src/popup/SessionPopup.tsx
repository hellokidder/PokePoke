import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { motion } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { useT } from "../i18n/context";
import SourceIcon from "../icons/SourceIcon";
import { formatPopupShortcutModPrefix } from "../lib/shortcutModifiers";
import { cn } from "../lib/utils";
import { sourceLabel } from "../panel/sessionPanelHelpers";
import type { Session } from "../types";
import { PopupCloseButton } from "./PopupCloseButton";
import { PopupShortcutBadge } from "./PopupShortcutBadge";
import {
  entryOffsetX,
  glowGradientFor,
  POPUP_BODY_CLASS,
  POPUP_CARD_CLASS,
  POPUP_SHELL_CLASS,
  POPUP_SHELL_MINI_CLASS,
  type PopupPosition,
  parseSessionIdFromLabel,
  readHashParam,
  readPosition,
} from "./popupShared";

function useTimeAgo() {
  const t = useT();
  return (dateStr: string): string => {
    const now = Date.now();
    const then = new Date(dateStr).getTime();
    const seconds = Math.floor((now - then) / 1000);
    if (seconds < 60) return t("time.just_now");
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return t("time.minutes_ago", { n: minutes });
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return t("time.hours_ago", { n: hours });
    const days = Math.floor(hours / 24);
    return t("time.days_ago", { n: days });
  };
}

export function SessionPopup() {
  const t = useT();
  const timeAgo = useTimeAgo();
  const [session, setSession] = useState<Session | null>(null);
  const [visible, setVisible] = useState(false);
  const [position] = useState<PopupPosition>(readPosition);
  const isMini = readHashParam("size") === "mini";
  const [shortcutN, setShortcutN] = useState<number | null>(() => {
    const raw = readHashParam("n");
    if (raw === null) return null;
    const n = parseInt(raw, 10);
    return Number.isFinite(n) && n >= 0 && n < 9 ? n : null;
  });
  const [shortcutModPrefix, setShortcutModPrefix] = useState("\u2318");

  useEffect(() => {
    const loadShortcutModifiers = () => {
      invoke<{ popup_shortcut_modifiers?: string }>("get_settings")
        .then((data) => {
          setShortcutModPrefix(
            formatPopupShortcutModPrefix(data.popup_shortcut_modifiers ?? "CmdOrCtrl"),
          );
        })
        .catch(() => {
          setShortcutModPrefix("\u2318");
        });
    };
    loadShortcutModifiers();
    const unlisten = listen("settings-updated", loadShortcutModifiers);
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  useEffect(() => {
    const label = getCurrentWebviewWindow().label;
    const id = parseSessionIdFromLabel(label) ?? "";

    invoke<Session | null>("get_session_by_id", { id }).then((data) => {
      if (data) {
        setSession(data);
        setVisible(true);
      }
    });
  }, []);

  const sessionRef = useRef<Session | null>(null);
  sessionRef.current = session;

  const triggeredRef = useRef(false);
  const handleClick = async () => {
    if (triggeredRef.current) return;
    const s = sessionRef.current;
    if (!s) return;
    triggeredRef.current = true;
    try {
      if (s.terminal_tty || s.workspace_path) {
        await invoke("focus_session_terminal", { id: s.id });
      }
      setVisible(false);
      await invoke("close_popup_window", { id: s.id });
    } catch (_e) {
      triggeredRef.current = false;
    }
  };
  const handleClickRef = useRef(handleClick);
  handleClickRef.current = handleClick;

  const handleDismiss = async () => {
    if (triggeredRef.current) return;
    const s = sessionRef.current;
    if (!s) return;
    triggeredRef.current = true;
    try {
      setVisible(false);
      await invoke("close_popup_window", { id: s.id });
    } catch (_e) {
      triggeredRef.current = false;
    }
  };

  useEffect(() => {
    const label = getCurrentWebviewWindow().label;
    const unlistenP = listen<Array<{ label: string; n: number }>>(
      "popup-shortcuts-updated",
      (event) => {
        const mine = event.payload.find((m) => m.label === label);
        if (!mine) {
          setShortcutN(null);
          return;
        }
        setShortcutN(mine.n >= 0 && mine.n < 9 ? mine.n : null);
      },
    );
    const unlistenT = getCurrentWebviewWindow().listen("popup-shortcut-trigger", () => {
      void handleClickRef.current();
    });
    return () => {
      void unlistenP.then((f) => f());
      void unlistenT.then((f) => f());
    };
  }, []);

  if (!session) {
    return <div className={POPUP_SHELL_CLASS} />;
  }

  const glowKind: "idle" | "pending" | "last_failed" | "default" =
    session.status === "idle"
      ? "idle"
      : session.status === "pending"
        ? "pending"
        : session.status === "last_failed"
          ? "last_failed"
          : "default";
  const canFocus = !!(session.terminal_tty || session.workspace_path);

  const displayMessage = (() => {
    if (session.status !== "last_failed") return session.message;
    const KNOWN_REASONS = [
      "rate_limit",
      "overloaded",
      "server_error",
      "authentication_failed",
      "oauth_org_not_allowed",
      "billing_error",
      "invalid_request",
      "model_not_found",
      "max_output_tokens",
    ];
    const reason =
      session.failure_reason && KNOWN_REASONS.includes(session.failure_reason)
        ? session.failure_reason
        : "unknown";
    const localized = t(`stop_failure.${reason}`);
    const detail = session.message.replace(/^API Error:\s*/i, "").trim();
    if (reason === "unknown" && detail) return detail;
    return localized;
  })();

  const offsetX = entryOffsetX(position);

  if (isMini) {
    return (
      <motion.div
        className={cn(POPUP_SHELL_MINI_CLASS, "cursor-pointer")}
        onClick={handleClick}
        initial={{ opacity: 0, x: offsetX, scale: 0.95 }}
        animate={visible ? { opacity: 1, x: 0, scale: 1 } : { opacity: 0, x: offsetX, scale: 0.95 }}
        transition={{ type: "spring", stiffness: 320, damping: 28, mass: 0.6 }}
      >
        <div
          className="pointer-events-none absolute inset-[-1px] z-0 rounded-[14px]"
          style={{ background: glowGradientFor(glowKind) }}
        />
        <div className="relative z-[1] flex h-full flex-col rounded-[14px] panel-base px-2.5 py-1.5 text-[var(--text-primary)]">
          <PopupCloseButton
            onClick={() => void handleDismiss()}
            ariaLabel={t("popup.close")}
            title={t("popup.close")}
            inline
            compact
          />
          <div className="flex min-h-0 flex-1 items-center gap-1.5 pr-6">
            <div className="flex shrink-0 items-center">
              <div className="scale-[0.5] origin-center inline-block -m-1">
                <SourceIcon
                  source={session.source}
                  status={session.status}
                  colorSeed={session.task_id}
                />
              </div>
            </div>
            <span className="flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[11px] font-semibold leading-tight tracking-[0.01em]">
              {session.title}
            </span>
            {session.source && (
              <span className="popup-pill whitespace-nowrap rounded-full px-1.5 py-[1px] text-[9px] uppercase leading-none tracking-[0.03em]">
                {sourceLabel(session)}
              </span>
            )}
            {shortcutN !== null && (
              <PopupShortcutBadge
                shortcutModPrefix={shortcutModPrefix}
                shortcutN={shortcutN}
                compact
              />
            )}
          </div>
        </div>
      </motion.div>
    );
  }

  return (
    <motion.div
      className={cn(POPUP_SHELL_CLASS, "cursor-pointer")}
      onClick={handleClick}
      initial={{ opacity: 0, x: offsetX, scale: 0.95 }}
      animate={visible ? { opacity: 1, x: 0, scale: 1 } : { opacity: 0, x: offsetX, scale: 0.95 }}
      transition={{ type: "spring", stiffness: 320, damping: 28, mass: 0.6 }}
    >
      <PopupCloseButton
        onClick={() => void handleDismiss()}
        ariaLabel={t("popup.close")}
        title={t("popup.close")}
      />
      <div className={POPUP_CARD_CLASS}>
        <div
          className="pointer-events-none absolute inset-[-1px] z-0 rounded-[14px]"
          style={{ background: glowGradientFor(glowKind) }}
        />
        <div className={POPUP_BODY_CLASS}>
          <div className="flex min-h-0 flex-1 gap-3">
            <div className="flex items-start pt-0.5">
              <SourceIcon
                source={session.source}
                status={session.status}
                colorSeed={session.task_id}
              />
            </div>
            <div className="flex min-w-0 flex-1 flex-col">
              <div className="mb-1.5 flex items-center gap-2">
                <span className="flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[14px] font-semibold tracking-[0.01em]">
                  {session.title}
                </span>
                {session.source && (
                  <span className="popup-pill whitespace-nowrap rounded-full px-2 py-[2px] text-[10px] uppercase tracking-[0.04em]">
                    {sourceLabel(session)}
                  </span>
                )}
              </div>
              <div className="flex-1 overflow-hidden text-[12.5px] leading-[1.5] text-[var(--text-secondary)] [-webkit-box-orient:vertical] [-webkit-line-clamp:3] [display:-webkit-box]">
                {displayMessage}
              </div>
            </div>
          </div>
          <div className="mt-2 flex items-center justify-between">
            <span className="text-[11px] text-[var(--text-tertiary)]">
              {timeAgo(session.updated_at)}
            </span>
            <span className="inline-flex items-center gap-2">
              {canFocus && (
                <span className="text-[11px] tracking-[0.02em] text-[var(--text-tertiary)]">
                  {t("popup.click_to_jump")}
                </span>
              )}
              {shortcutN !== null && (
                <PopupShortcutBadge shortcutModPrefix={shortcutModPrefix} shortcutN={shortcutN} />
              )}
            </span>
          </div>
        </div>
      </div>
    </motion.div>
  );
}
