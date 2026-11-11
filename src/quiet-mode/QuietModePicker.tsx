import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AnimatePresence, motion } from "framer-motion";
import { BellOff, X } from "lucide-react";
import { useT } from "../i18n/context";
import { cn } from "../lib/utils";
import { Button } from "../components/ui/button";
import { DEFAULT_QUIET_MODE_SHORTCUT, formatFullShortcut } from "../lib/shortcutModifiers";
import type { QuietModeStatus } from "./types";
import { QuietModeProgressRing } from "./QuietModeProgressRing";

const DURATION_PRESETS = [5, 15, 30, 60] as const;

const IDLE_STATUS: QuietModeStatus = {
  active: false,
  remaining_secs: null,
  total_secs: null,
};

function readInitialStatus(): QuietModeStatus {
  if (typeof window === "undefined") return IDLE_STATUS;
  const params = new URLSearchParams(window.location.hash.slice(1));
  if (params.get("active") === "1") {
    return {
      active: true,
      remaining_secs: Number(params.get("remaining") ?? 0),
      total_secs: Number(params.get("total") ?? 0),
    };
  }
  return IDLE_STATUS;
}

function sameQuietStatus(a: QuietModeStatus, b: QuietModeStatus): boolean {
  return (
    a.active === b.active && a.remaining_secs === b.remaining_secs && a.total_secs === b.total_secs
  );
}

export default function QuietModePicker() {
  const t = useT();
  const [status, setStatus] = useState<QuietModeStatus>(readInitialStatus);
  const [durationMins, setDurationMins] = useState(30);
  const [busy, setBusy] = useState(false);
  const [hotkeyLabel, setHotkeyLabel] = useState(() =>
    formatFullShortcut(DEFAULT_QUIET_MODE_SHORTCUT),
  );
  // 窗体刚获得焦点时 WKWebView 可能在光标下合成 mousedown,短暂禁用点击避免白闪
  const [pointerReady, setPointerReady] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const presetRefs = useRef<(HTMLButtonElement | null)[]>([]);

  const applyStatus = (next: QuietModeStatus) => {
    setStatus((prev) => (sameQuietStatus(prev, next) ? prev : next));
  };

  const refresh = () => {
    void invoke<QuietModeStatus>("get_quiet_mode_status").then(applyStatus);
    void invoke<{ quiet_mode_duration_mins?: number; quiet_mode_shortcut?: string }>(
      "get_settings",
    ).then((s) => {
      if (s.quiet_mode_duration_mins && s.quiet_mode_duration_mins > 0) {
        setDurationMins(s.quiet_mode_duration_mins);
      }
      setHotkeyLabel(formatFullShortcut(s.quiet_mode_shortcut ?? DEFAULT_QUIET_MODE_SHORTCUT));
    });
  };

  useEffect(() => {
    document.documentElement.classList.add("quiet-mode-window");
    const frame = window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => setPointerReady(true));
    });
    return () => {
      window.cancelAnimationFrame(frame);
      document.documentElement.classList.remove("quiet-mode-window");
    };
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only quiet mode sync
  useEffect(() => {
    refresh();
    const unlistenChanged = listen<QuietModeStatus>("quiet-mode-changed", (e) => {
      applyStatus(e.payload);
    });
    const unlistenSettings = listen("settings-updated", () => {
      void invoke<{ quiet_mode_shortcut?: string }>("get_settings").then((s) => {
        setHotkeyLabel(formatFullShortcut(s.quiet_mode_shortcut ?? DEFAULT_QUIET_MODE_SHORTCUT));
      });
    });
    const tick = window.setInterval(() => {
      void invoke<QuietModeStatus>("get_quiet_mode_status").then(applyStatus);
    }, 1000);
    return () => {
      unlistenChanged.then((fn) => fn());
      unlistenSettings.then((fn) => fn());
      window.clearInterval(tick);
    };
  }, []);

  const close = () => {
    void invoke("close_quiet_mode_picker");
  };

  const start = async () => {
    setBusy(true);
    try {
      await invoke("start_quiet_mode", { durationMins });
    } finally {
      setBusy(false);
    }
  };

  const cancel = async () => {
    setBusy(true);
    try {
      await invoke("cancel_quiet_mode");
    } finally {
      setBusy(false);
    }
  };

  const remaining = status.remaining_secs ?? 0;
  const total = status.total_secs ?? remaining;
  const isActive = status.active && status.remaining_secs != null;
  // 当前选中的预设下标(时长可能来自设置、不在预设里,兜底聚焦第一个)
  const selectedPresetIndex = DURATION_PRESETS.findIndex((mins) => mins === durationMins);
  const focusIndex = selectedPresetIndex >= 0 ? selectedPresetIndex : 0;

  // 这个窗口是"打开就能按"的工具面板:焦点必须落在时长选项上,
  // 这样上下左右能直接换时长、回车直接开始,不用先移鼠标。
  // 依赖 isActive:进行中→未开启(cancel)后要重新聚焦。
  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在状态切换时重新聚焦
  useEffect(() => {
    const focusTarget = () => {
      if (isActive) {
        cancelRef.current?.focus();
        return;
      }
      // 只有在「焦点还停在 body」或「焦点仍在时长选项组里」时才重新聚焦:
      // 前者是窗口刚拿到 key 的重试,后者是因为设置异步加载完成后选中项变了
      // (默认 30 → 设置里的 15),焦点必须跟着选中项走,否则会出现
      // 「15 是选中框、30 是焦点框」两张卡片同时高亮。
      // 焦点在「开始」/关闭等其它控件上时不动,避免抢用户操作。
      const active = document.activeElement;
      const focusInPresets = presetRefs.current.some((el) => el !== null && el === active);
      if (active && active !== document.body && !focusInPresets) return;
      presetRefs.current[focusIndex]?.focus();
    };

    focusTarget();
    const frame = window.requestAnimationFrame(focusTarget);
    const retries = [60, 180, 400].map((ms) => window.setTimeout(focusTarget, ms));
    window.addEventListener("focus", focusTarget);
    return () => {
      window.cancelAnimationFrame(frame);
      retries.forEach((id) => window.clearTimeout(id));
      window.removeEventListener("focus", focusTarget);
    };
  }, [isActive, focusIndex]);

  // 回车开始 / Esc 关闭。焦点已经落在主按钮上时由按钮自身的 click 处理,
  // 这里只在焦点不在按钮上(点到空白、刚打开还没渲染完)时兜底,避免触发两次。
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close();
        return;
      }
      if (event.key !== "Enter") return;
      const tag = (event.target as HTMLElement | null)?.tagName;
      if (tag === "BUTTON" || tag === "INPUT") return;
      if (isActive || busy) return;
      event.preventDefault();
      void start();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  // 2x2 网格的方向键语义:左右在本行内移动,上下跨行移动;回车/空格直接开始。
  // 这里 preventDefault 掉 Enter,避免按钮原生 click 再触发一次 start。
  const moveSelection = (nextIndex: number) => {
    setDurationMins(DURATION_PRESETS[nextIndex]);
    presetRefs.current[nextIndex]?.focus();
  };

  const onGridKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const i = focusIndex;
    switch (event.key) {
      case "ArrowLeft":
        if (i % 2 === 1) {
          event.preventDefault();
          moveSelection(i - 1);
        }
        return;
      case "ArrowRight":
        if (i % 2 === 0 && i + 1 < DURATION_PRESETS.length) {
          event.preventDefault();
          moveSelection(i + 1);
        }
        return;
      case "ArrowUp":
        if (i - 2 >= 0) {
          event.preventDefault();
          moveSelection(i - 2);
        }
        return;
      case "ArrowDown":
        if (i + 2 < DURATION_PRESETS.length) {
          event.preventDefault();
          moveSelection(i + 2);
        }
        return;
      case "Enter":
      case " ":
        event.preventDefault();
        if (!busy) void start();
        return;
      default:
        return;
    }
  };

  return (
    <div className="relative h-full w-full bg-transparent p-[14px] text-[var(--text-primary)]">
      <div
        className={cn(
          "quiet-picker-card relative flex h-full flex-col overflow-hidden rounded-[14px]",
          !pointerReady && "pointer-events-none",
        )}
      >
        <div
          data-tauri-drag-region
          className="absolute left-0 top-0 z-10 h-9 w-[300px] cursor-move"
          aria-hidden="true"
        />
        <div
          className="pointer-events-none absolute inset-x-0 top-0 h-28"
          style={{
            background:
              "radial-gradient(ellipse 75% 100% at 50% 0%, color-mix(in srgb, var(--brand-blue) 12%, transparent), transparent 70%)",
          }}
          aria-hidden="true"
        />
        <div className="relative flex items-start justify-between border-b border-[var(--divider)] px-5 pb-3.5 pt-4">
          <div className="flex min-w-0 items-start gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-[var(--brand-blue)]/25 bg-gradient-to-br from-[var(--brand-blue)]/20 to-[var(--brand-purple)]/10 text-[var(--brand-blue)] shadow-[inset_0_1px_0_var(--inset-highlight)]">
              <BellOff className="h-[18px] w-[18px]" strokeWidth={1.75} aria-hidden="true" />
            </div>
            <div className="min-w-0 pt-0.5">
              <h1 className="m-0 text-[15px] font-bold tracking-[0.02em] brand-gradient-text">
                {t("quiet_mode.title")}
              </h1>
              <p className="mt-1 text-[11px] leading-relaxed text-[var(--text-secondary)]">
                {t("quiet_mode.subtitle")}
              </p>
            </div>
          </div>
          <button
            type="button"
            data-tauri-no-drag
            className="quiet-interactive-btn rounded-md p-1 text-[var(--text-secondary)] hover:bg-[var(--surface-hover)] hover:text-[var(--text-primary)]"
            onClick={close}
            aria-label={t("quiet_mode.close")}
          >
            <X className="h-4 w-4" strokeWidth={1.75} />
          </button>
        </div>

        <div className="relative flex min-h-0 flex-1 flex-col px-5 py-4">
          <AnimatePresence mode="wait" initial={false}>
            {isActive ? (
              <motion.div
                key="active"
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -4 }}
                transition={{ duration: 0.2 }}
                className="flex flex-1 flex-col items-center justify-center gap-3 py-1"
              >
                <div className="flex items-center gap-2">
                  <span className="relative flex h-2 w-2">
                    <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-[var(--brand-blue)] opacity-40" />
                    <span className="relative inline-flex h-2 w-2 rounded-full bg-[var(--brand-blue)]" />
                  </span>
                  <span className="text-[11px] font-medium tracking-wide text-[var(--brand-blue)]">
                    {t("quiet_mode.active_label")}
                  </span>
                </div>

                <QuietModeProgressRing remainingSecs={remaining} totalSecs={total} size="lg" />

                <p className="max-w-[240px] text-center text-[11px] leading-relaxed text-[var(--text-secondary)]">
                  {t("quiet_mode.active_hint")}
                </p>
              </motion.div>
            ) : (
              <motion.div
                key="idle"
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -4 }}
                transition={{ duration: 0.2 }}
                className="flex flex-1 flex-col gap-3"
              >
                <p className="text-[11px] font-medium tracking-wide text-[var(--text-tertiary)]">
                  {t("quiet_mode.pick_duration")}
                </p>
                <div
                  role="radiogroup"
                  aria-label={t("quiet_mode.pick_duration")}
                  className="grid grid-cols-2 gap-2"
                  onKeyDown={onGridKeyDown}
                >
                  {DURATION_PRESETS.map((mins, index) => {
                    const selected = durationMins === mins;
                    return (
                      <button
                        key={mins}
                        ref={(el) => {
                          presetRefs.current[index] = el;
                        }}
                        type="button"
                        role="radio"
                        aria-checked={selected}
                        tabIndex={selected ? 0 : -1}
                        data-tauri-no-drag
                        className={cn(
                          "quiet-interactive-btn group relative flex flex-col items-center justify-center rounded-xl border px-3 py-2.5 transition-colors outline-none",
                          "focus-visible:ring-2 focus-visible:ring-[var(--brand-blue)]/45 focus-visible:ring-offset-0",
                          selected
                            ? "border-[var(--brand-blue)]/50 bg-gradient-to-b from-[var(--brand-blue)]/14 to-[var(--brand-purple)]/6 shadow-[0_0_0_1px_color-mix(in_srgb,var(--brand-blue)_18%,transparent)]"
                            : "border-[var(--border-color)] bg-[var(--surface-muted)] hover:border-[var(--border-hover)] hover:bg-[var(--surface-hover)]",
                        )}
                        onClick={() => setDurationMins(mins)}
                      >
                        <span
                          className={cn(
                            "text-xl font-semibold tabular-nums leading-none tracking-tight",
                            selected
                              ? "text-[var(--brand-blue)]"
                              : "text-[var(--text-primary)] group-hover:text-[var(--text-primary)]",
                          )}
                        >
                          {mins}
                        </span>
                        <span
                          className={cn(
                            "mt-1 text-[10px] font-medium",
                            selected
                              ? "text-[var(--brand-blue)]/80"
                              : "text-[var(--text-tertiary)]",
                          )}
                        >
                          {t("quiet_mode.duration_unit")}
                        </span>
                      </button>
                    );
                  })}
                </div>
              </motion.div>
            )}
          </AnimatePresence>

          <div className="mt-auto space-y-3 pt-2">
            {isActive ? (
              <Button
                ref={cancelRef}
                className="quiet-interactive-btn h-10 w-full border-[var(--border-color)] bg-[var(--surface-muted)] text-[var(--text-primary)] hover:bg-[var(--surface-hover)] focus-visible:ring-0"
                variant="outline"
                disabled={busy}
                onClick={() => void cancel()}
              >
                {t("quiet_mode.end_now")}
              </Button>
            ) : (
              <Button
                className="quiet-interactive-btn h-10 w-full bg-gradient-to-r from-[var(--brand-purple)] to-[var(--brand-blue)] text-white shadow-[0_4px_14px_color-mix(in_srgb,var(--brand-blue)_28%,transparent)] hover:opacity-95 focus-visible:ring-0"
                disabled={busy}
                onClick={() => void start()}
              >
                {t("quiet_mode.start")}
              </Button>
            )}

            <p className="text-center text-[10px] text-[var(--text-tertiary)]">
              <kbd className="popup-kbd rounded px-1.5 py-0.5 font-mono text-[10px] font-semibold">
                {hotkeyLabel}
              </kbd>
              <span className="ml-1.5">{t("quiet_mode.hotkey_hint_suffix")}</span>
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
