import SourceIcon from "../icons/SourceIcon";
import type { getLandingCopy } from "./landingCopy";
import { Eyebrow } from "./LandingShared";

type IntegrationsCopy = ReturnType<typeof getLandingCopy>["integrations"];

/**
 * 集成网格。重设计后(20260814)去掉 per-slug 渐变 overlay 与发光 badge:
 * 品牌色只由 SourceIcon 承载(界面语言),卡片保持平铺灰阶。
 */
export default function IntegrationBento({ copy }: { copy: IntegrationsCopy }) {
  return (
    <section id="integrations" className="px-5 py-24 sm:px-8 lg:px-12">
      <div className="mx-auto max-w-6xl">
        <div className="mb-14 max-w-3xl">
          <Eyebrow text={copy.eyebrow} />
          <h2 className="mt-5 text-4xl font-semibold tracking-[-0.03em] text-white sm:text-5xl">
            {copy.title}
          </h2>
          <p className="mt-5 text-lg leading-relaxed text-white/55">{copy.body}</p>
        </div>

        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {copy.items.map((item) => (
            <article
              key={item.slug}
              className="flex h-full min-h-[168px] flex-col rounded-2xl border border-white/[0.08] bg-white/[0.02] p-5 transition hover:border-white/[0.16] hover:bg-white/[0.03]"
            >
              <div className="flex items-start justify-between gap-3">
                <div className="flex items-center gap-3">
                  {/* 不传 colorSeed：集成网格要品牌色（SOURCE_COLORS），
                      按 slug 哈希出来的颜色会让 Codex/omp 撞成同一色。
                      应用内的会话列表/弹窗才用 colorSeed=task_id 做每会话配色。 */}
                  <SourceIcon source={item.slug} status="running" animate={false} />
                  <div>
                    <h3 className="text-lg font-semibold text-white">{item.name}</h3>
                    <p className="mt-0.5 text-sm text-white/50">{item.tone}</p>
                  </div>
                </div>
                <code className="shrink-0 rounded-md border border-white/[0.08] px-2 py-1 font-mono text-[10px] uppercase tracking-[0.08em] text-white/40">
                  {copy.badge}
                </code>
              </div>
              <p className="mt-4 line-clamp-2 font-mono text-[11px] leading-relaxed text-white/30">
                {item.events}
              </p>
            </article>
          ))}
        </div>

        {/* 9 个接入端正好 3×3；「更多接入中」不再占第 10 张卡，改成网格下面的一行说明 */}
        <p className="mt-8 text-center text-sm text-white/35">{copy.moreLabel}</p>
      </div>
    </section>
  );
}
