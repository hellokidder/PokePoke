import type { getLandingCopy } from "./landingCopy";
import { Eyebrow, SectionHeader } from "./LandingShared";

type TerminalJumpCopy = ReturnType<typeof getLandingCopy>["terminalJump"];

const TAB_DEMO = [
  { label: "zsh", active: false },
  { label: "vite dev", active: false },
  { label: "pokepoke", active: true },
  { label: "tail -f", active: false },
];

/**
 * 终端跳转区。重设计后(20260814)去掉点阵/径向光晕/琥珀发光:
 * ⌘N 键帽保留物理质感(它是"触觉"叙事的一部分),但不再发光;
 * active tab 与键帽 N 用信号红标记。
 */
export default function TerminalEcosystem({ copy }: { copy: TerminalJumpCopy }) {
  return (
    <section className="px-5 py-24 sm:px-8 lg:px-12">
      <div className="mx-auto max-w-6xl">
        <SectionHeader eyebrow={copy.eyebrow} title={copy.title} body={copy.body} />

        <div className="grid gap-5 lg:grid-cols-2 lg:items-stretch">
          {/* 终端演示 */}
          <div className="flex h-full flex-col rounded-2xl border border-white/[0.08] bg-[#0b0b0e] p-6 sm:p-8">
            <div className="overflow-hidden rounded-xl border border-white/[0.08] bg-[#08080a]">
              <div className="flex items-center gap-3 border-b border-white/[0.06] px-4 py-3">
                <div className="flex gap-1.5">
                  <span className="size-2.5 rounded-full bg-[#ff5f57]" />
                  <span className="size-2.5 rounded-full bg-[#febc2e]" />
                  <span className="size-2.5 rounded-full bg-[#28c840]" />
                </div>
                <span className="ml-2 font-mono text-[11px] text-white/40">
                  Ghostty — ~/kidder/poke-poke
                </span>
              </div>
              <div className="flex items-center gap-1.5 overflow-x-auto border-b border-white/[0.06] px-3 py-2">
                {TAB_DEMO.map((tab) => (
                  <span
                    key={tab.label}
                    className={
                      tab.active
                        ? "inline-flex items-center gap-1.5 rounded-md border border-[#ff453a]/40 bg-[#ff453a]/[0.08] px-2.5 py-1 font-mono text-[11px] text-white/85"
                        : "inline-flex items-center rounded-md border border-white/[0.05] px-2.5 py-1 font-mono text-[11px] text-white/40"
                    }
                  >
                    {tab.active ? <span className="size-1.5 rounded-full bg-[#ff453a]" /> : null}
                    {tab.label}
                  </span>
                ))}
              </div>
              <div className="space-y-1 px-5 py-4 font-mono text-[12px] leading-relaxed">
                <div className="text-white/40">
                  $ <span className="text-white/70">pnpm tauri dev</span>
                </div>
                <div className="text-white/40">✓ compiled in 2.3s</div>
                <div className="text-white/40">
                  ▶ waiting on <span className="text-white/60">http://localhost:1420</span>
                </div>
                <div className="flex items-center gap-1 pt-1">
                  <span className="inline-block h-3 w-1.5 animate-pulse bg-white/40" />
                </div>
              </div>
            </div>

            <div className="mt-8 flex flex-1 flex-col items-center justify-center">
              <div
                className="flex items-center justify-center rounded-xl border border-white/10 px-8 py-4 font-mono text-2xl font-semibold tracking-[0.08em] text-white sm:text-3xl"
                style={{
                  background:
                    "linear-gradient(180deg, rgba(255,255,255,0.09) 0%, rgba(255,255,255,0.02) 45%, rgba(0,0,0,0.35) 100%)",
                  boxShadow:
                    "inset 0 1px 0 rgba(255,255,255,0.14), inset 0 -2px 0 rgba(0,0,0,0.5), 0 6px 0 rgba(0,0,0,0.55)",
                }}
              >
                <span className="text-white/55">⌘</span>
                <span className="mx-2 text-white/20">+</span>
                <span className="text-[#ff453a]">N</span>
              </div>
              <p className="mt-6 max-w-sm text-center text-sm leading-relaxed text-white/45">
                {copy.shortcutLabel}
              </p>
            </div>
          </div>

          {/* 跳转目标清单 */}
          <div className="flex h-full flex-col rounded-2xl border border-white/[0.08] bg-white/[0.02] p-6 sm:p-8">
            <Eyebrow text={copy.worksWithLabel} />

            <div className="flex flex-1 flex-col items-center justify-center py-8">
              <div className="flex w-fit items-center gap-3 rounded-xl border border-white/[0.1] bg-white/[0.03] px-5 py-3">
                <span aria-hidden="true" className="size-2 bg-[#ff453a]" />
                <span className="font-semibold text-white">PokePoke</span>
                <span className="font-mono text-[10px] uppercase tracking-[0.14em] text-white/40">
                  jump router
                </span>
              </div>
            </div>

            <div className="mt-auto space-y-6">
              {copy.categories.map((category) => (
                <div key={category.label}>
                  <div className="mb-3 flex items-center gap-3">
                    <span className="font-mono text-[10px] uppercase tracking-[0.18em] text-white/30">
                      {category.label}
                    </span>
                    <span className="h-px flex-1 bg-white/[0.08]" />
                  </div>
                  <div className="flex flex-wrap gap-2">
                    {category.items.map((name) => (
                      <span
                        key={name}
                        className="rounded-lg border border-white/[0.08] bg-white/[0.02] px-3.5 py-2 font-mono text-[12px] text-white/60 transition hover:border-white/[0.16] hover:text-white/85"
                      >
                        {name}
                      </span>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
