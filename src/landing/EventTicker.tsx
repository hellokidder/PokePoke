/**
 * EventTicker — hero 下方的 hook 事件名跑马灯。
 * 内容即产品机制:滚动的是各接入端真实 fire 的事件名(SessionStart /
 * permission.ask / agent_settled …),不是装饰性 lorem。
 * 双份内容 + translateX(-50%) 实现无缝循环;reduced-motion 由全局规则冻结。
 */
function TickerItems({ items }: { items: string[] }) {
  return (
    <>
      {items.map((item) => (
        <span key={item} className="flex items-center gap-8">
          <span>{item}</span>
          <span aria-hidden="true" className="size-1 bg-[#ff453a]/60" />
        </span>
      ))}
    </>
  );
}

export default function EventTicker({ items }: { items: string[] }) {
  return (
    <div className="overflow-hidden border-y border-white/[0.08] bg-white/[0.015] py-3">
      <div className="landing-ticker-track flex w-max items-center gap-8 whitespace-nowrap font-mono text-[11px] uppercase tracking-[0.14em] text-white/30">
        <TickerItems items={items} />
        <TickerItems items={items} />
      </div>
    </div>
  );
}
