/**
 * Landing 共享小组件。设计原则(docs/briefs/20260814-landing-redesign.md):
 * 无装饰性动效、无发光胶囊;eyebrow 用像素方块小点,向像素八爪鱼签名对齐。
 */

import { useEffect } from "react";
import type { Locale } from "../i18n/strings";

/** landing / docs 共用的文档外壳:lang + landing-document 背景类(+ 可选标题) */
export function useLandingDocumentShell(locale: Locale, title?: string) {
  useEffect(() => {
    const previousLang = document.documentElement.lang;
    const previousTitle = document.title;
    document.documentElement.lang = locale === "zh" ? "zh-CN" : "en";
    document.documentElement.classList.add("landing-document");
    document.body.classList.add("landing-document");
    if (title) document.title = title;
    return () => {
      document.documentElement.lang = previousLang;
      document.title = previousTitle;
      document.documentElement.classList.remove("landing-document");
      document.body.classList.remove("landing-document");
    };
  }, [locale, title]);
}

export function Eyebrow({ text }: { text: string }) {
  return (
    <div className="inline-flex items-center gap-2.5">
      <span aria-hidden="true" className="size-1.5 bg-[#ff453a]" />
      <span className="font-mono text-xs uppercase tracking-[0.14em] text-white/45">{text}</span>
    </div>
  );
}

export function SectionHeader({
  eyebrow,
  title,
  body,
}: {
  eyebrow: string;
  title: string;
  body?: string;
}) {
  return (
    <div className="mb-14 flex flex-col justify-between gap-4 md:flex-row md:items-end">
      <div>
        <Eyebrow text={eyebrow} />
        <h2 className="mt-5 text-4xl font-semibold tracking-[-0.03em] text-white sm:text-5xl">
          {title}
        </h2>
      </div>
      {body ? <p className="max-w-xl text-lg leading-relaxed text-white/50">{body}</p> : null}
    </div>
  );
}

/** 分区发线 + mono 编号——给页面一副看得见的工程骨架 */
export function SectionDivider({ index }: { index: string }) {
  return (
    <div aria-hidden="true" className="px-5 sm:px-8 lg:px-12">
      <div className="mx-auto flex max-w-6xl items-center gap-4 border-t border-white/[0.08] pt-3">
        <span className="font-mono text-[10px] uppercase tracking-[0.2em] text-white/25">
          {index}
        </span>
        <span className="size-1 bg-[#ff453a]/50" />
      </div>
    </div>
  );
}
