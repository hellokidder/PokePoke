import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { useEffect, useState } from "react";
import SourceIcon from "../icons/SourceIcon";
import { POPUP_BODY_CLASS, glowGradientFor } from "../popup/popupShared";
import type { getLandingCopy } from "./landingCopy";

type StickyPokeCopy = ReturnType<typeof getLandingCopy>["stickyPoke"];

const DISMISS_KEY = "pokepoke-sticky-dismissed";
/** 滚过约一屏后出现 */
const TRIGGER_RATIO = 0.9;
/** 驻留后的轻微"戳一下"间隔(ms) */
const NUDGE_INTERVAL = 16000;

/**
 * 页面级"赖皮 popup"(docs/briefs/20260814-landing-redesign.md):
 * 滚过 hero 后在右下角弹出并驻留,不点不走——页面本身演示产品核心机制。
 * 点击主体 = 平滑滚回顶部下载区(网页版 ⌘N 跳回);点 × = 本次会话不再出现。
 * 视觉上 1:1 复用真实 popup 的 class 与弹簧参数;reduced-motion 下静止呈现。
 */
export default function StickyPoke({ copy }: { copy: StickyPokeCopy }) {
  const reduced = useReducedMotion();
  const [pastHero, setPastHero] = useState(false);
  const [dismissed, setDismissed] = useState(() => {
    try {
      return sessionStorage.getItem(DISMISS_KEY) === "1";
    } catch {
      return false;
    }
  });
  const [nudgeKey, setNudgeKey] = useState(0);

  useEffect(() => {
    const onScroll = () => {
      setPastHero(window.scrollY > window.innerHeight * TRIGGER_RATIO);
    };
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => window.removeEventListener("scroll", onScroll);
  }, []);

  const visible = pastHero && !dismissed;

  useEffect(() => {
    if (!visible || reduced) return;
    const timer = window.setInterval(() => setNudgeKey((k) => k + 1), NUDGE_INTERVAL);
    return () => window.clearInterval(timer);
  }, [visible, reduced]);

  const dismiss = () => {
    setDismissed(true);
    try {
      sessionStorage.setItem(DISMISS_KEY, "1");
    } catch {
      /* 私密模式下写不进去就算了,本次会话内 state 已生效 */
    }
  };

  const handleJumpBack = () => {
    window.scrollTo({ top: 0, behavior: reduced ? "auto" : "smooth" });
    dismiss();
  };

  return (
    <AnimatePresence>
      {visible ? (
        <motion.div
          key="sticky-poke"
          className="fixed bottom-5 right-5 z-50 hidden w-[320px] sm:block"
          initial={reduced ? { opacity: 1 } : { opacity: 0, x: 30, scale: 0.95 }}
          animate={{ opacity: 1, x: 0, scale: 1 }}
          exit={reduced ? { opacity: 0 } : { opacity: 0, x: 30, scale: 0.95 }}
          transition={{ type: "spring", stiffness: 320, damping: 28, mass: 0.6 }}
        >
          {/* 驻留期间偶尔轻轻"戳"一下边缘求关注(克制版人格化) */}
          <motion.div
            key={nudgeKey}
            animate={reduced ? undefined : { x: [0, -5, 0, -2.5, 0] }}
            transition={{ duration: 0.45, ease: "easeOut" }}
          >
            <div className="relative">
              <button
                type="button"
                aria-label={copy.closeLabel}
                title={copy.closeLabel}
                onClick={dismiss}
                className="absolute -left-2 -top-2 z-10 flex size-5 items-center justify-center rounded-full border border-white/[0.12] bg-[#1a1a1e] text-[11px] leading-none text-white/60 transition hover:bg-[#26262b] hover:text-white"
              >
                ×
              </button>
              <button
                type="button"
                onClick={handleJumpBack}
                className="relative block w-full cursor-pointer overflow-hidden rounded-[14px] text-left"
              >
                <div
                  aria-hidden="true"
                  className="pointer-events-none absolute inset-[-1px] z-0 rounded-[14px]"
                  style={{ background: glowGradientFor("pending") }}
                />
                <div className={POPUP_BODY_CLASS}>
                  <div className="flex min-h-0 flex-1 gap-3">
                    <div className="flex items-start pt-0.5">
                      <SourceIcon
                        source="cursor"
                        status={copy.status}
                        colorSeed={copy.colorSeed}
                        animate={false}
                      />
                    </div>
                    <div className="flex min-w-0 flex-1 flex-col">
                      <div className="mb-1.5 flex items-center gap-2">
                        <span className="flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[14px] font-semibold tracking-[0.01em]">
                          {copy.title}
                        </span>
                        <span className="popup-pill whitespace-nowrap rounded-full px-2 py-[2px] text-[10px] uppercase tracking-[0.04em]">
                          {copy.sourcePill}
                        </span>
                      </div>
                      <div className="flex-1 overflow-hidden text-[12.5px] leading-[1.5] text-[var(--text-secondary)]">
                        {copy.body}
                      </div>
                    </div>
                  </div>
                  <div className="mt-2 flex items-center justify-between">
                    <span className="text-[11px] text-[var(--text-tertiary)]">{copy.time}</span>
                    <span className="inline-flex items-center gap-2 text-[11px] tracking-[0.02em] text-[var(--text-tertiary)]">
                      {copy.jump}
                      <span className="popup-kbd rounded-md px-1.5 py-[1px] text-[10px] font-semibold tracking-[0.04em]">
                        {copy.badge}
                      </span>
                    </span>
                  </div>
                </div>
              </button>
            </div>
            <p className="mt-2 pr-1 text-right font-mono text-[10px] text-white/30">{copy.note}</p>
          </motion.div>
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}
