import trayIcon from "../icons/tray-default.png";
import type { Locale } from "../i18n/strings";
import type { getLandingCopy } from "./landingCopy";
import { downloadLatestDmg } from "./downloadLatest";

type NavCopy = ReturnType<typeof getLandingCopy>["nav"];
type LanguageCopy = ReturnType<typeof getLandingCopy>["language"];

/**
 * Landing 顶导航。重设计后(20260814)去掉玻璃胶囊:纯平铺,
 * 唯一的强视觉元素是红色 Download CTA。
 */
export default function LandingNav({
  copy,
  language,
  locale,
  onLocaleChange,
}: {
  copy: NavCopy;
  language: LanguageCopy;
  locale: Locale;
  onLocaleChange: (locale: Locale) => void;
}) {
  const nextLocale: Locale = locale === "en" ? "zh" : "en";

  return (
    <nav className="relative z-10 mx-auto flex max-w-6xl items-center justify-between py-6">
      <a
        className="flex items-center gap-3 font-semibold tracking-tight text-white"
        href="#top"
        aria-label="PokePoke home"
      >
        <img className="size-8 rounded-xl" src={trayIcon} alt="" />
        <span>PokePoke</span>
      </a>
      <div className="hidden items-center gap-7 text-sm text-white/50 md:flex">
        <a className="transition hover:text-white/90" href="#features">
          {copy.features}
        </a>
        <a className="transition hover:text-white/90" href="#integrations">
          {copy.integrations}
        </a>
        <a className="transition hover:text-white/90" href="#compare">
          {copy.compare}
        </a>
        <a className="transition hover:text-white/90" href="/docs">
          {copy.docs}
        </a>
        <a className="transition hover:text-white/90" href="/privacy.html">
          {copy.privacy}
        </a>
      </div>
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
          {copy.download}
        </button>
      </div>
    </nav>
  );
}
