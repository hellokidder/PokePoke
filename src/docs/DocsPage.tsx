import trayIcon from "../icons/tray-default.png";
import type { Locale } from "../i18n/strings";
import { getLandingCopy } from "../landing/landingCopy";
import { downloadLatestDmg } from "../landing/downloadLatest";
import { useLandingDocumentShell } from "../landing/LandingShared";
import type { DocsBlock } from "./docsCopy";
import { getDocsCopy } from "./docsCopy";

/**
 * /docs — 用户手册页(man-page 气质:mono 编号 + 发线骨架,无装饰)。
 * 内容全部来自 src/docs/docsCopy{En,Zh}.ts;事实源见 docsCopy.ts 头注释。
 * 路由:App.tsx 在非 Tauri 分支按 location.pathname 前缀 /docs 门控;
 * CF Pages 直下 /docs 由 public/_redirects 的 SPA fallback 兜底。
 */

/** 行内格式:`code` 与 **bold** */
function rich(text: string) {
  const seen = new Map<string, number>();
  return text.split(/(\*\*[^*]+\*\*|`[^`]+`)/g).map((part) => {
    const n = seen.get(part) ?? 0;
    seen.set(part, n + 1);
    const key = `${part.slice(0, 24)}#${n}`;
    if (part.startsWith("**") && part.endsWith("**")) {
      return (
        <strong key={key} className="font-medium text-white/85">
          {part.slice(2, -2)}
        </strong>
      );
    }
    if (part.startsWith("`") && part.endsWith("`")) {
      return (
        <code
          key={key}
          className="border border-white/[0.1] bg-white/[0.05] px-1.5 py-0.5 font-mono text-[0.85em] text-white/80"
        >
          {part.slice(1, -1)}
        </code>
      );
    }
    return part;
  });
}

function Block({ block }: { block: DocsBlock }) {
  switch (block.kind) {
    case "para":
      return <p className="leading-relaxed text-white/60">{rich(block.text)}</p>;
    case "list":
      return (
        <ul className="flex flex-col gap-2.5">
          {block.items.map((item) => (
            <li key={item.slice(0, 32)} className="flex items-start gap-3 text-white/60">
              <span aria-hidden="true" className="mt-[9px] size-1 shrink-0 bg-[#ff453a]" />
              <span className="leading-relaxed">{rich(item)}</span>
            </li>
          ))}
        </ul>
      );
    case "code":
      return (
        <pre className="overflow-x-auto border border-white/[0.1] bg-black/50 p-4 font-mono text-[13px] leading-relaxed text-white/75">
          {block.lines.join("\n")}
        </pre>
      );
    case "table":
      return (
        <div className="overflow-x-auto border border-white/[0.1]">
          <table className="w-full border-collapse text-sm">
            <thead>
              <tr className="border-b border-white/[0.1] bg-white/[0.03]">
                {block.head.map((h) => (
                  <th
                    key={h}
                    className="px-4 py-2.5 text-left font-mono text-[10px] uppercase tracking-[0.14em] text-white/40"
                  >
                    {h}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((row) => (
                <tr key={row[0]} className="border-b border-white/[0.06] last:border-0">
                  {row.map((cell, ci) => (
                    <td
                      // biome-ignore lint/suspicious/noArrayIndexKey: 列序稳定
                      key={ci}
                      className={`px-4 py-2.5 align-top ${
                        ci === 0 ? "font-medium text-white/80" : "text-white/55"
                      }`}
                    >
                      {rich(cell)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case "note":
      return (
        <div className="border-l-2 border-[#ff453a] bg-white/[0.03] px-4 py-3 leading-relaxed text-white/60">
          {rich(block.text)}
        </div>
      );
  }
}

export default function DocsPage({
  locale,
  onLocaleChange,
}: {
  locale: Locale;
  onLocaleChange: (locale: Locale) => void;
}) {
  const copy = getDocsCopy(locale);
  const language = getLandingCopy(locale).language;
  useLandingDocumentShell(locale, `PokePoke — ${copy.header.manual}`);

  const nextLocale: Locale = locale === "en" ? "zh" : "en";

  return (
    <main className="dark min-h-screen bg-[#050506] font-sans text-white antialiased">
      {/* ===== 顶栏 ===== */}
      <header className="sticky top-0 z-20 border-b border-white/[0.08] bg-[#050506]/90 backdrop-blur-sm">
        <div className="mx-auto flex max-w-6xl items-center justify-between px-5 py-4 sm:px-8">
          <a href="/" className="flex items-center gap-3 font-semibold tracking-tight text-white">
            <img className="size-7 rounded-lg" src={trayIcon} alt="" />
            <span>PokePoke</span>
            <span className="font-mono text-[10px] uppercase tracking-[0.18em] text-white/35">
              /docs
            </span>
          </a>
          <div className="flex items-center gap-2.5">
            <button
              className="rounded-lg border border-white/[0.1] px-3 py-2 text-sm font-medium text-white/60 transition hover:border-white/[0.2] hover:text-white"
              type="button"
              aria-label={language.label}
              onClick={() => onLocaleChange(nextLocale)}
            >
              {language.current} / {language.next}
            </button>
            <button
              type="button"
              onClick={() => void downloadLatestDmg()}
              className="rounded-lg bg-[#ff453a] px-4 py-2 text-sm font-semibold text-white transition hover:bg-[#ff5f57]"
            >
              {copy.header.download}
            </button>
          </div>
        </div>
      </header>

      {/* ===== man-page 头 ===== */}
      <div className="border-b border-white/[0.08] px-5 py-12 sm:px-8">
        <div className="mx-auto max-w-6xl">
          <p className="font-mono text-xs uppercase tracking-[0.18em] text-[#ff453a]">
            {copy.header.manual}
          </p>
          <p className="mt-4 max-w-xl text-lg leading-relaxed text-white/55">
            {copy.header.tagline}
          </p>
        </div>
      </div>

      {/* ===== 目录 + 正文 ===== */}
      <div className="mx-auto grid max-w-6xl gap-10 px-5 py-12 sm:px-8 lg:grid-cols-[200px_1fr] lg:gap-16">
        <aside className="lg:sticky lg:top-24 lg:self-start">
          <nav className="flex flex-wrap gap-x-5 gap-y-2 lg:flex-col lg:gap-2.5">
            {copy.sections.map((s) => (
              <a
                key={s.id}
                href={`#${s.id}`}
                className="group flex items-baseline gap-2.5 text-sm text-white/45 transition hover:text-white"
              >
                <span className="font-mono text-[10px] text-white/25 group-hover:text-[#ff453a]">
                  {s.index}
                </span>
                {s.title}
              </a>
            ))}
          </nav>
        </aside>

        <article className="flex max-w-3xl flex-col gap-14">
          {copy.sections.map((s) => (
            <section key={s.id} id={s.id} className="scroll-mt-24">
              <h2 className="flex items-baseline gap-3 border-b border-white/[0.08] pb-3 text-2xl font-semibold tracking-[-0.02em] text-white">
                <span className="font-mono text-sm font-normal text-[#ff453a]">{s.index}</span>
                {s.title}
              </h2>
              <div className="mt-6 flex flex-col gap-5">
                {s.blocks.map((b, bi) => (
                  // biome-ignore lint/suspicious/noArrayIndexKey: 区块序稳定
                  <Block key={bi} block={b} />
                ))}
              </div>
            </section>
          ))}
        </article>
      </div>

      {/* ===== 底栏 ===== */}
      <footer className="border-t border-white/[0.08] px-5 py-8 sm:px-8">
        <div className="mx-auto flex max-w-6xl items-center justify-between font-mono text-[11px] uppercase tracking-[0.14em] text-white/30">
          <a href="/" className="transition hover:text-white/70">
            ← {copy.header.backHome}
          </a>
          <span>PokePoke © 2026 Kidder</span>
        </div>
      </footer>
    </main>
  );
}
