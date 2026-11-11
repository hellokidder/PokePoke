import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { Button } from "../components/ui/button";
import { useT } from "../i18n/context";
import { cn } from "../lib/utils";
import { PopupCloseButton } from "./PopupCloseButton";
import {
  entryOffsetX,
  glowGradientFor,
  POPUP_BODY_CLASS,
  POPUP_CARD_CLASS,
  POPUP_SHELL_CLASS,
  type PopupPosition,
  readHashParam,
  readPosition,
} from "./popupShared";

type UpdatePhase = "idle" | "downloading" | "installing" | "error";

interface PendingUpdateInfo {
  version: string;
  current_version: string;
  notes: string | null;
  content_length: number | null;
}

interface UpdateProgressPayload {
  downloaded: number;
  total: number | null;
  phase: "downloading" | "download_finished";
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function UpdatePopup() {
  const t = useT();
  const [position] = useState<PopupPosition>(readPosition);
  const [visible, setVisible] = useState(false);
  const [phase, setPhase] = useState<UpdatePhase>("idle");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [info, setInfo] = useState<PendingUpdateInfo | null>(null);
  const [downloaded, setDownloaded] = useState(0);
  const [totalBytes, setTotalBytes] = useState<number | null>(null);
  const triggeredRef = useRef(false);
  const version = readHashParam("version") ?? info?.version ?? "?";

  useEffect(() => {
    setVisible(true);
    invoke<PendingUpdateInfo>("get_pending_update_info")
      .then((i) => setInfo(i))
      .catch(() => setInfo(null));
  }, []);

  const handleInstall = async () => {
    if (triggeredRef.current) return;
    triggeredRef.current = true;
    setErrorMsg(null);
    setDownloaded(0);
    setTotalBytes(null);

    let unlisten: (() => void) | null = null;
    let pendingRaf: number | null = null;
    let lastDownloaded = 0;
    let lastTotal: number | null = null;
    try {
      unlisten = await listen<UpdateProgressPayload>("update-progress", (e) => {
        if (e.payload.phase === "downloading") {
          lastDownloaded = e.payload.downloaded;
          lastTotal = e.payload.total;
          if (pendingRaf === null) {
            pendingRaf = requestAnimationFrame(() => {
              pendingRaf = null;
              setDownloaded(lastDownloaded);
              setTotalBytes(lastTotal);
            });
          }
        }
      });

      setPhase("downloading");
      await invoke("download_update_with_progress");
      if (pendingRaf !== null) {
        cancelAnimationFrame(pendingRaf);
        pendingRaf = null;
      }
      setDownloaded(lastDownloaded);
      setTotalBytes(lastTotal);

      setPhase("installing");
      await invoke("install_cached_update");
    } catch (e) {
      triggeredRef.current = false;
      setErrorMsg(String(e));
      setPhase("error");
    } finally {
      if (pendingRaf !== null) cancelAnimationFrame(pendingRaf);
      if (unlisten) unlisten();
    }
  };

  const handleRetry = () => {
    triggeredRef.current = false;
    setPhase("idle");
    setErrorMsg(null);
  };

  const handleSkip = async () => {
    setVisible(false);
    await invoke("mark_update_skipped", { version });
  };

  const handleDismiss = async () => {
    setVisible(false);
    await invoke("dismiss_update_popup", { version });
  };

  const busy = phase === "downloading" || phase === "installing";
  const progressPercent =
    totalBytes && totalBytes > 0
      ? Math.min(100, Math.floor((downloaded / totalBytes) * 100))
      : null;

  const offsetX = entryOffsetX(position);

  return (
    <motion.div
      className={POPUP_SHELL_CLASS}
      initial={{ opacity: 0, x: offsetX, scale: 0.95 }}
      animate={visible ? { opacity: 1, x: 0, scale: 1 } : { opacity: 0, x: offsetX, scale: 0.95 }}
      transition={{ type: "spring", stiffness: 320, damping: 28, mass: 0.6 }}
    >
      <PopupCloseButton
        onClick={() => void handleDismiss()}
        disabled={busy}
        ariaLabel={t("popup.update_dismiss")}
        title={t("popup.update_dismiss")}
      />
      <div className={POPUP_CARD_CLASS}>
        <div
          className="pointer-events-none absolute inset-[-1px] z-0 rounded-[14px]"
          style={{ background: glowGradientFor("update") }}
        />
        <div className={POPUP_BODY_CLASS}>
          <div className="flex min-h-0 flex-1 gap-3">
            <div className="flex min-w-0 flex-1 flex-col">
              <div className="mb-1 flex items-center gap-2">
                <span className="flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[14px] font-semibold tracking-[0.01em]">
                  {t("popup.update_title", { version })}
                </span>
              </div>
              {/* 下载 / 安装阶段这块位置让给进度条:此时"点击立即升级"已经没意义,
                  且正文列只有 ~66px,两样同时出现会把说明挤到 10px 高(重新触发裁字)。 */}
              {!busy && (
                <div className="overflow-hidden text-[12.5px] leading-[1.45] text-[var(--text-secondary)] [-webkit-box-orient:vertical] [-webkit-line-clamp:2] [display:-webkit-box]">
                  {t("popup.update_message")}
                </div>
              )}
              {phase === "downloading" && (
                <div className="mt-2">
                  <div className="popup-progress-track h-1 w-full overflow-hidden rounded-[2px]">
                    <div
                      className={cn(
                        "h-full rounded-[2px] bg-[linear-gradient(90deg,rgba(168,85,247,0.95),rgba(236,72,153,0.95))] transition-[width] duration-[120ms] ease-out",
                        progressPercent === null &&
                          "w-[40%] animate-[popup-update-indeterminate_1.2s_ease-in-out_infinite]",
                      )}
                      style={
                        progressPercent !== null ? { width: `${progressPercent}%` } : undefined
                      }
                    />
                  </div>
                  <div className="mt-1 text-[11px] tracking-[0.02em] text-[var(--text-secondary)]">
                    {progressPercent !== null
                      ? t("popup.update_downloading_pct", { pct: progressPercent })
                      : t("popup.update_downloading_bytes", { bytes: formatBytes(downloaded) })}
                  </div>
                </div>
              )}
              {phase === "installing" && (
                <div className="mt-2">
                  <div className="popup-progress-track h-1 w-full overflow-hidden rounded-[2px]">
                    <div className="h-full w-[40%] animate-[popup-update-indeterminate_1.2s_ease-in-out_infinite] rounded-[2px] bg-[linear-gradient(90deg,rgba(168,85,247,0.95),rgba(236,72,153,0.95))]" />
                  </div>
                  <div className="mt-1 text-[11px] tracking-[0.02em] text-[var(--text-secondary)]">
                    {t("popup.update_installing_restart")}
                  </div>
                </div>
              )}
            </div>
          </div>
          {errorMsg && (
            <div
              className="mt-1.5 cursor-help overflow-hidden text-ellipsis whitespace-nowrap text-[11px] tracking-[0.02em] text-red-400/95"
              title={errorMsg}
            >
              {t("popup.update_failed")}
            </div>
          )}
          {/* 查看更新说明挪到按钮行左侧：正文列只有 ~66px 高，留在这里会把
              说明文字挤到 12px（需要 19px）—— 就是「文字下半截被裁掉」那个 bug。 */}
          <div className="mt-2 flex items-center justify-between gap-2">
            <button
              type="button"
              className={cn(
                "min-w-0 truncate border-0 bg-transparent p-0 text-[11px] tracking-[0.02em] text-[var(--brand-purple)] underline",
                "hover:opacity-80",
                "disabled:cursor-default disabled:opacity-40 disabled:hover:text-purple-400",
              )}
              onClick={() =>
                openUrl(`https://github.com/hellokidder/PokePoke/releases/tag/v${version}`)
              }
              disabled={busy}
            >
              {t("popup.update_notes_show")}
            </button>
            <div className="flex shrink-0 items-center gap-2">
              <Button
                variant="secondary"
                size="sm"
                className="surface-btn h-7 text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                onClick={handleSkip}
                disabled={busy}
              >
                {t("popup.update_skip")}
              </Button>
              <Button
                size="sm"
                className="h-7 border border-purple-500 bg-purple-500/85 text-white shadow-[0_0_12px_-2px_rgba(168,85,247,0.6)] hover:bg-purple-500"
                onClick={phase === "error" ? handleRetry : handleInstall}
                disabled={busy}
              >
                {phase === "downloading"
                  ? t("popup.update_downloading")
                  : phase === "installing"
                    ? t("popup.update_installing")
                    : phase === "error"
                      ? t("popup.update_retry")
                      : t("popup.update_install_now")}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </motion.div>
  );
}
