import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import SourceIcon from "../icons/SourceIcon";
import trayIcon from "../icons/tray-default.png";
import { POPUP_BODY_CLASS, glowGradientFor } from "../popup/popupShared";
import type { getLandingCopy } from "./landingCopy";

type DemoCopy = ReturnType<typeof getLandingCopy>["hero"]["demo"];

/**
 * Hero 仿真桌面 demo(docs/briefs/20260814-landing-redesign.md):
 * 不用截图/录像,DOM 级模拟一个 macOS 桌面切片——菜单栏(含 PokePoke tray
 * 图标与角标)、终端窗口、PokePoke panel,按 4 相位循环演示产品核心机制:
 *
 *   P0 平静   —— 各 agent 会话在 panel 里安静运行
 *   P1 戳你   —— northwind 变 pending,tray 出现红角标,popup 弹入并驻留
 *   P2 点击   —— 仿真光标移动到 popup 上按下
 *   P3 跳回   —— popup 收走,焦点落回会话所在的终端窗口(红框标记)
 *
 * popup 的入场弹簧参数与真实 popup(SessionPopup.tsx)一致;卡片复用真实
 * class(panel-base / popup-pill / popup-kbd 经 popupShared 常量),由外层
 * landing 容器的 .dark 作用域提供 CSS 变量。
 * prefers-reduced-motion:静止在 P1(popup 驻留),不循环。
 */

const PHASE_DURATION = [2600, 3000, 1100, 2400];
/** 光标静止位(相对 frame 的比例) */
const CURSOR_REST = { x: 0.38, y: 0.62 };

function DemoCursor({
  frameRef,
  phase,
  popupRef,
}: {
  frameRef: React.RefObject<HTMLDivElement | null>;
  phase: number;
  popupRef: React.RefObject<HTMLDivElement | null>;
}) {
  const [pos, setPos] = useState({ x: 0, y: 0 });

  useEffect(() => {
    const frame = frameRef.current;
    if (!frame) return;
    const f = frame.getBoundingClientRect();
    if (phase === 2 && popupRef.current) {
      const p = popupRef.current.getBoundingClientRect();
      setPos({ x: p.left + p.width / 2 - f.left, y: p.top + p.height / 2 - f.top });
    } else {
      setPos({ x: f.width * CURSOR_REST.x, y: f.height * CURSOR_REST.y });
    }
  }, [phase, frameRef, popupRef]);

  return (
    <motion.div
      aria-hidden="true"
      className="pointer-events-none absolute left-0 top-0 z-30"
      animate={{ x: pos.x, y: pos.y, scale: phase === 2 ? 0.82 : 1 }}
      transition={{ type: "spring", stiffness: 260, damping: 24 }}
    >
      <svg
        aria-hidden="true"
        width="18"
        height="22"
        viewBox="0 0 16 20"
        fill="none"
        className="-translate-x-0.5 -translate-y-0.5"
      >
        <path
          d="M1 1L1 15L5 11L9 19L12 17.5L8 10L14 10L1 1Z"
          fill="white"
          stroke="black"
          strokeWidth="1"
        />
      </svg>
    </motion.div>
  );
}

export default function HeroDesktop({ copy }: { copy: DemoCopy }) {
  const reduced = useReducedMotion();
  const [phase, setPhase] = useState(0);
  const frameRef = useRef<HTMLDivElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (reduced) {
      setPhase(1);
      return;
    }
    let cancelled = false;
    let current = 0;
    let timer = 0;
    const tick = () => {
      if (cancelled) return;
      current = (current + 1) % PHASE_DURATION.length;
      setPhase(current);
      timer = window.setTimeout(tick, PHASE_DURATION[current]);
    };
    timer = window.setTimeout(tick, PHASE_DURATION[0]);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [reduced]);

  const poking = phase === 1 || phase === 2;
  const jumped = phase === 3;

  return (
    <div
      ref={frameRef}
      className="relative overflow-hidden rounded-2xl border border-white/[0.08] bg-[#0b0b0e] shadow-[0_24px_80px_rgba(0,0,0,0.45)]"
    >
      {/* macOS 菜单栏 */}
      <div className="flex items-center justify-between border-b border-white/[0.06] px-4 py-2 text-[11px] text-white/40">
        <div className="flex items-center gap-3.5">
          <span className="font-semibold text-white/70">Code</span>
          <span className="hidden sm:inline">File</span>
          <span className="hidden sm:inline">Edit</span>
          <span className="hidden sm:inline">Window</span>
          <span className="hidden sm:inline">Help</span>
        </div>
        <div className="flex items-center gap-3">
          <span className="relative inline-flex">
            <img className="size-4 rounded" src={trayIcon} alt="" />
            {poking ? (
              <span className="absolute -right-1.5 -top-1.5 flex h-3.5 min-w-3.5 items-center justify-center rounded-full bg-[#ff453a] px-0.5 text-[8px] font-bold leading-none text-white">
                1
              </span>
            ) : null}
          </span>
          <span className="font-mono">09:41</span>
        </div>
      </div>

      <div className="grid gap-3 p-3 md:grid-cols-[1.15fr_1fr]">
        {/* 终端窗口 —— P3 跳回时被红框标记 */}
        <div
          className={`hidden flex-col overflow-hidden rounded-xl border bg-[#08080a] transition-colors duration-300 md:flex ${
            jumped ? "border-[#ff453a]/50" : "border-white/[0.08]"
          }`}
        >
          <div className="flex items-center gap-3 border-b border-white/[0.06] px-3.5 py-2.5">
            <div className="flex gap-1.5">
              <span className="size-2.5 rounded-full bg-[#ff5f57]" />
              <span className="size-2.5 rounded-full bg-[#febc2e]" />
              <span className="size-2.5 rounded-full bg-[#28c840]" />
            </div>
            <span className="font-mono text-[10.5px] text-white/35">
              Ghostty — ~/projects/northwind
            </span>
          </div>
          <div className="flex items-center gap-1.5 border-b border-white/[0.06] px-3 py-1.5">
            {["zsh", "claude", "logs"].map((tab) => (
              <span
                key={tab}
                className={`inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1 font-mono text-[10.5px] ${
                  tab === "claude"
                    ? jumped
                      ? "border-[#ff453a]/40 bg-[#ff453a]/[0.08] text-white/85"
                      : "border-white/[0.1] bg-white/[0.04] text-white/70"
                    : "border-white/[0.05] text-white/35"
                }`}
              >
                {tab === "claude" && jumped ? (
                  <span className="size-1.5 rounded-full bg-[#ff453a]" />
                ) : null}
                {tab}
              </span>
            ))}
          </div>
          <div className="space-y-1.5 px-4 py-3.5 font-mono text-[11px] leading-relaxed">
            <div className="text-white/35">
              $ <span className="text-white/65">claude</span>
            </div>
            <div className="text-white/35">✻ Refactoring auth module…</div>
            {poking ? (
              <div className="text-white/70">? Allow Edit src/auth.ts? (y/n)</div>
            ) : (
              <div className="text-white/35">✓ 3 files updated</div>
            )}
            <div className="flex items-center pt-0.5">
              <span
                className={`inline-block h-3 w-1.5 ${
                  poking ? "animate-pulse bg-[#ff453a]/80" : "bg-white/30"
                }`}
              />
            </div>
          </div>
        </div>

        {/* PokePoke panel —— 行结构与真实 SessionPanelCard 对齐 */}
        <div className="flex flex-col overflow-hidden rounded-xl border border-white/[0.08] bg-[#0d0d10]">
          <div className="flex items-center justify-between border-b border-white/[0.06] px-3.5 py-2.5">
            <div className="flex items-center gap-2">
              <img className="size-4 rounded" src={trayIcon} alt="" />
              <span className="text-[12px] font-semibold text-white/85">PokePoke</span>
            </div>
            <span className="text-[10px] text-white/35">⌘</span>
          </div>
          <div className="flex-1 space-y-2 p-2.5">
            {copy.sessions.map((session) => {
              const isNorthwind = session.project === "northwind";
              const status = isNorthwind ? (poking ? "pending" : "running") : session.status;
              const message = isNorthwind && !poking ? copy.runningMessage : session.message;
              return (
                <div
                  key={session.project}
                  className={`flex items-center gap-2.5 rounded-lg border px-2.5 py-2 transition-colors duration-300 ${
                    isNorthwind && poking
                      ? "border-[#ff453a]/30 bg-[#ff453a]/[0.05]"
                      : "border-white/[0.06] bg-white/[0.02]"
                  }`}
                >
                  <span className="inline-flex shrink-0 scale-[0.62]">
                    <SourceIcon
                      source={session.source}
                      status={status}
                      colorSeed={session.colorSeed}
                      animate={false}
                    />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="flex items-center justify-between gap-2">
                      <span className="truncate text-[12px] font-semibold leading-4 text-white/90">
                        {session.project}
                      </span>
                      <span className="popup-pill shrink-0 rounded-full px-1.5 py-[1px] text-[8px] uppercase leading-none tracking-[0.06em]">
                        {session.sourcePill}
                      </span>
                    </span>
                    <span className="mt-0.5 block truncate text-[10.5px] text-white/45">
                      {message}
                    </span>
                  </span>
                </div>
              );
            })}
          </div>
          <div className="flex items-center justify-between border-t border-white/[0.06] px-3.5 py-2 text-[10px] text-white/35">
            <span>{copy.notificationToggle}</span>
            <span>{copy.footerCount}</span>
          </div>
        </div>
      </div>

      {/* popup —— 结构与真实 SessionPopup 对齐,弹入后驻留直到被"点击" */}
      <AnimatePresence>
        {poking ? (
          <motion.div
            className="absolute bottom-3 right-3 z-20 w-[min(300px,78%)]"
            initial={{ opacity: 0, x: 30, scale: 0.95 }}
            animate={{ opacity: 1, x: 0, scale: 1 }}
            exit={{ opacity: 0, x: 30, scale: 0.95 }}
            transition={{ type: "spring", stiffness: 320, damping: 28, mass: 0.6 }}
          >
            <div ref={popupRef} className="relative overflow-hidden rounded-[14px]">
              <div
                aria-hidden="true"
                className="pointer-events-none absolute inset-[-1px] z-0 rounded-[14px]"
                style={{ background: glowGradientFor("pending") }}
              />
              <div className={POPUP_BODY_CLASS}>
                <div className="flex min-h-0 flex-1 gap-3">
                  <div className="flex items-start pt-0.5">
                    <SourceIcon
                      source={copy.popup.source}
                      status={copy.popup.status}
                      colorSeed={copy.popup.colorSeed}
                      animate={false}
                    />
                  </div>
                  <div className="flex min-w-0 flex-1 flex-col">
                    <div className="mb-1.5 flex items-center gap-2">
                      <span className="flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[14px] font-semibold tracking-[0.01em]">
                        {copy.popup.title}
                      </span>
                      <span className="popup-pill whitespace-nowrap rounded-full px-2 py-[2px] text-[10px] uppercase tracking-[0.04em]">
                        {copy.popup.sourcePill}
                      </span>
                    </div>
                    <div className="flex-1 overflow-hidden text-[12.5px] leading-[1.5] text-[var(--text-secondary)]">
                      {copy.popup.body}
                    </div>
                  </div>
                </div>
                <div className="mt-2 flex items-center justify-between">
                  <span className="text-[11px] text-[var(--text-tertiary)]">{copy.popup.time}</span>
                  <span className="inline-flex items-center gap-2 text-[11px] tracking-[0.02em] text-[var(--text-tertiary)]">
                    {copy.popup.jump}
                    <span className="popup-kbd rounded-md px-1.5 py-[1px] text-[10px] font-semibold tracking-[0.04em]">
                      {copy.popup.badge}
                    </span>
                  </span>
                </div>
              </div>
            </div>
          </motion.div>
        ) : null}
      </AnimatePresence>

      {!reduced ? <DemoCursor frameRef={frameRef} phase={phase} popupRef={popupRef} /> : null}
    </div>
  );
}
