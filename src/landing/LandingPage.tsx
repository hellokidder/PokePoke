import type { ReactNode } from "react";
import type { Locale } from "../i18n/strings";
import { getLandingCopy } from "./landingCopy";
import { downloadLatestDmg } from "./downloadLatest";
import CodeBackdrop from "./CodeBackdrop";
import HeroDesktop from "./HeroDesktop";
import EventTicker from "./EventTicker";
import IntegrationBento from "./IntegrationBento";
import LandingNav from "./LandingNav";
import PixelOctopus from "./PixelOctopus";
import { Eyebrow, SectionHeader, useLandingDocumentShell } from "./LandingShared";
import StickyPoke from "./StickyPoke";
import TerminalEcosystem from "./TerminalEcosystem";

/**
 * Landing 编排(20260814 重设计,docs/briefs/20260814-landing-redesign.md):
 * 墨黑底 + 单一信号红 + 灰阶;无粒子/玻璃/glow/scroll-reveal/展示字体。
 * 两个有信息含义的动效全部来自产品机制本身:
 *   1. HeroDesktop —— 会话状态流转 + popup 弹入驻留 + 光标点击跳回(循环)
 *   2. StickyPoke —— 页面级"不点不走"popup,点了滚回顶部(网页版 ⌘N)
 * 根容器挂 .dark:让复用的真实组件 class(panel-base/popup-pill/popup-kbd)
 * 解析到深色 CSS 变量。
 */

type LandingPageProps = {
  locale: Locale;
  onLocaleChange: (locale: Locale) => void;
};

/**
 * 内容分区卡片：每块分区一块独立磨砂玻璃，块与块之间留缝，让整页代码背景从缝隙
 * 里透出来（而不是一个盖满全页的大面板）。
 * index 沿用原来 SectionDivider 的编号，挪到卡片右上角，保持编辑感。
 */
function Panel({ children, index }: { children: ReactNode; index?: string }) {
  return (
    <div className="relative rounded-[26px] border border-white/[0.07] bg-white/[0.012] shadow-[inset_0_1px_0_rgba(255,255,255,0.06)] backdrop-blur-[9px]">
      {index ? (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute right-6 top-6 flex items-center gap-2 font-mono text-[10px] uppercase tracking-[0.2em] text-white/25"
        >
          <span className="size-1 bg-[#ff453a]/50" />
          {index}
        </div>
      ) : null}
      {children}
    </div>
  );
}

function LandingPage({ locale, onLocaleChange }: LandingPageProps) {
  const copy = getLandingCopy(locale);

  useLandingDocumentShell(locale);

  return (
    <main className="landing-page dark min-h-screen overflow-x-hidden bg-[#050506] text-[#e8e9ed]">
      {/* 整页代码背景：固定层、pointer-events-none，不参与布局也不拦事件 */}
      <CodeBackdrop />

      {/* 内容区磨砂玻璃：代码背景只从四周留白和玻璃背后透出来，
          不直接压在正文上；backdrop-blur 让背后的聚光变成柔和光斑。 */}
      <div className="relative z-10 mx-auto flex max-w-[1180px] flex-col gap-6 p-3 sm:gap-8 sm:p-5 lg:p-6">
        {/* ===== NAV（独立一条，不和 hero 挤在同一块里） ===== */}
        <Panel>
          <div className="px-5 sm:px-8 lg:px-12">
            <LandingNav
              copy={copy.nav}
              language={copy.language}
              locale={locale}
              onLocaleChange={onLocaleChange}
            />
          </div>
        </Panel>

        {/* ===== HERO ===== */}
        <Panel>
          <section id="top" className="relative px-5 sm:px-8 lg:px-12">
            <div className="relative z-10 mx-auto grid max-w-6xl items-center gap-14 pb-20 pt-10 lg:grid-cols-[0.9fr_1.1fr] lg:gap-16 lg:pb-28 lg:pt-14">
              <div className="max-w-2xl">
                <Eyebrow text={copy.hero.eyebrow} />
                <h1 className="mt-7 text-balance text-5xl font-bold leading-[1.04] tracking-[-0.035em] text-white sm:text-6xl lg:text-[4rem]">
                  {copy.hero.title}
                  <span aria-hidden="true" className="landing-cursor" />
                </h1>
                <p className="mt-7 max-w-xl text-lg leading-relaxed text-white/55">
                  {copy.hero.body}
                </p>
                <div className="mt-10 flex flex-col gap-4 sm:flex-row sm:items-center">
                  <button
                    type="button"
                    onClick={() => void downloadLatestDmg()}
                    className="rounded-xl bg-[#ff453a] px-6 py-3.5 text-center text-base font-semibold text-white transition hover:bg-[#ff5f57]"
                  >
                    {copy.hero.primary}
                  </button>
                  <p className="text-sm text-white/40">{copy.hero.shortcut}</p>
                </div>
                <a
                  className="mt-9 inline-block opacity-80 transition hover:opacity-100"
                  href="https://www.producthunt.com/products/pokepoke-agent-attention-for-macos?embed=true&utm_source=badge-featured&utm_medium=badge&utm_campaign=badge-pokepoke"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  <img
                    alt="PokePoke - Poke you when your AI agent pokes back | Product Hunt"
                    width={250}
                    height={54}
                    src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1195077&theme=dark&t=1784033563420"
                  />
                </a>
              </div>

              {/* 巨型像素八爪鱼从 demo 桌面右下角"戳出来":跨框叠层 + 硬投影(像素质感) */}
              <div className="relative">
                <HeroDesktop copy={copy.hero.demo} />
                <PixelOctopus
                  size={208}
                  className="animate-float-y pointer-events-none absolute -bottom-32 -right-28 z-10 hidden [filter:drop-shadow(6px_6px_0_rgba(0,0,0,0.45))] sm:block"
                />
                <PixelOctopus
                  size={110}
                  className="animate-float-y pointer-events-none absolute -bottom-20 -right-8 z-10 [filter:drop-shadow(4px_4px_0_rgba(0,0,0,0.45))] sm:hidden"
                />
              </div>
            </div>
          </section>
        </Panel>

        <EventTicker
          items={[...new Set(copy.integrations.items.flatMap((i) => i.events.split(" / ")))]}
        />

        <Panel>
          {/* ===== PROBLEM ===== */}
          <section className="px-5 py-24 sm:px-8 lg:px-12">
            <div className="mx-auto max-w-6xl">
              <Eyebrow text={copy.problem.eyebrow} />
              <div className="mt-8 grid gap-8 lg:grid-cols-[0.9fr_1.1fr] lg:items-end">
                <h2 className="text-4xl font-semibold tracking-[-0.03em] text-white sm:text-5xl">
                  {copy.problem.title}
                </h2>
                <p className="text-lg leading-relaxed text-white/55">{copy.problem.body}</p>
              </div>
            </div>
          </section>
        </Panel>

        {/* ===== FEATURES ===== */}
        <Panel index="01">
          <section id="features" className="px-5 py-24 sm:px-8 lg:px-12">
            <div className="mx-auto max-w-6xl">
              <SectionHeader
                eyebrow={copy.features.eyebrow}
                title={copy.features.title}
                body={copy.features.body}
              />

              <div className="relative grid gap-4 md:grid-cols-4 md:items-stretch">
                <div
                  aria-hidden="true"
                  className="pointer-events-none absolute left-[12.5%] right-[12.5%] top-12 hidden h-px border-t border-dashed border-white/10 md:block"
                />
                {copy.features.loop.map((item, index) => (
                  <article
                    key={item.title}
                    className="relative z-10 flex h-full flex-col rounded-2xl border border-white/[0.08] bg-[#0b0b0e] p-6 transition hover:border-white/[0.16] lg:p-7"
                  >
                    <span className="mb-8 flex items-center gap-2 font-mono text-xs text-white/40">
                      <span aria-hidden="true" className="size-1.5 bg-[#ff453a]" />
                      {String(index + 1).padStart(2, "0")}
                    </span>
                    <h3 className="text-xl font-semibold text-white">{item.title}</h3>
                    <p className="mt-3 text-sm leading-relaxed text-white/50">{item.body}</p>
                  </article>
                ))}
              </div>
            </div>
          </section>
        </Panel>

        <Panel index="02">
          <IntegrationBento copy={copy.integrations} />
        </Panel>

        <Panel index="03">
          <TerminalEcosystem copy={copy.terminalJump} />
        </Panel>

        {/* ===== COMPARE ===== */}
        <Panel index="04">
          <section id="compare" className="px-5 py-24 sm:px-8 lg:px-12">
            <div className="mx-auto max-w-6xl">
              <SectionHeader
                eyebrow={copy.compare.eyebrow}
                title={copy.compare.title}
                body={copy.compare.body}
              />

              <div className="mx-auto max-w-3xl space-y-3">
                {copy.compare.points.map((point) => (
                  <div
                    key={point}
                    className="flex items-start gap-4 rounded-xl border border-white/[0.08] bg-white/[0.02] p-5"
                  >
                    <span aria-hidden="true" className="mt-1.5 size-1.5 shrink-0 bg-[#ff453a]" />
                    <p className="text-sm leading-relaxed text-white/70">{point}</p>
                  </div>
                ))}
              </div>

              <p className="mx-auto mt-12 max-w-3xl text-center text-sm leading-relaxed text-white/35">
                {copy.compare.footer}
              </p>
            </div>
          </section>
        </Panel>

        {/* ===== PRIVACY + OPEN SOURCE ===== */}
        <Panel index="05">
          <section id="privacy" className="px-5 pb-24 sm:px-8 lg:px-12">
            <div className="mx-auto grid max-w-6xl gap-5 lg:grid-cols-2">
              <div className="flex h-full flex-col rounded-2xl border border-white/[0.08] border-l-white/25 bg-white/[0.02] p-8 lg:p-10">
                <Eyebrow text={copy.privacy.eyebrow} />
                <h2 className="mt-5 text-4xl font-semibold tracking-[-0.03em] text-white">
                  {copy.privacy.title}
                </h2>
                <p className="mt-5 text-lg leading-relaxed text-white/55">
                  {copy.privacy.bodyStart}{" "}
                  <code className="font-mono text-white">~/.pokepoke/</code>
                  {copy.privacy.bodyEnd}
                </p>
                <ul className="mt-6 space-y-3">
                  {copy.privacy.points.map((p) => (
                    <li key={p} className="flex items-start gap-3 text-sm text-white/70">
                      <svg
                        className="mt-0.5 size-4 shrink-0 text-white/45"
                        fill="none"
                        viewBox="0 0 24 24"
                        stroke="currentColor"
                        strokeWidth={2}
                        aria-hidden="true"
                      >
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                        />
                      </svg>
                      {p}
                    </li>
                  ))}
                </ul>
              </div>

              <div
                id="opensource"
                className="flex h-full flex-col rounded-2xl border border-white/[0.08] border-l-[#ff453a]/50 bg-white/[0.02] p-8 lg:p-10"
              >
                <Eyebrow text={copy.opensource.eyebrow} />
                <h2 className="mt-5 text-4xl font-semibold tracking-[-0.03em] text-white">
                  {copy.opensource.title}
                </h2>
                <p className="mt-5 text-lg leading-relaxed text-white/55">{copy.opensource.body}</p>
              </div>
            </div>
          </section>
        </Panel>

        <Panel>
          <footer className="px-5 py-8 text-sm text-white/30 sm:px-8 lg:px-12">
            <div className="mx-auto flex max-w-6xl flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
              <div className="flex items-center gap-4">
                <span>{copy.footer.tagline}</span>
                <a
                  href="https://x.com/HelloKidder01"
                  target="_blank"
                  rel="noopener noreferrer me"
                  className="inline-flex h-7 w-7 items-center justify-center rounded-md text-white/40 transition hover:bg-white/[0.06] hover:text-white/80"
                >
                  <span className="sr-only">{copy.footer.x}</span>
                  <svg
                    viewBox="0 0 24 24"
                    width="14"
                    height="14"
                    fill="currentColor"
                    aria-hidden="true"
                  >
                    <path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z" />
                  </svg>
                </a>
              </div>
              <div className="flex flex-wrap gap-x-5 gap-y-2">
                <button
                  type="button"
                  onClick={() => void downloadLatestDmg()}
                  className="transition hover:text-white/70"
                >
                  {copy.footer.download}
                </button>
                <a
                  className="transition hover:text-white/70"
                  href="https://github.com/hellokidder/PokePoke"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  {copy.footer.github}
                </a>
                <a className="transition hover:text-white/70" href="/docs">
                  {copy.footer.docs}
                </a>
                <a className="transition hover:text-white/70" href="/privacy.html">
                  {copy.footer.privacy}
                </a>
                <a
                  className="transition hover:text-white/70"
                  href="mailto:hellokiddermail@gmail.com"
                >
                  {copy.footer.contact}
                </a>
              </div>
            </div>
          </footer>
        </Panel>
      </div>

      <StickyPoke copy={copy.stickyPoke} />
    </main>
  );
}

export default LandingPage;
