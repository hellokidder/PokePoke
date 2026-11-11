import { useEffect, useRef } from "react";

/**
 * 整页背景的「鼠标聚光揭示代码」效果。
 *
 * 三层（都在 fixed 容器里，pointer-events-none）:
 *   1. 暖光：跟随光标的一团极淡珊瑚色径向渐变，像一盏灯
 *   2. 底图层：3 列代码文本，单色、极淡，常驻当纹理
 *   3. 聚光层：同内容，但按语法上色（命令绿 / 日志灰 / 代码蓝 / 注释暗），
 *      被跟随光标的径向遮罩点亮 —— 看起来像"用光照亮一段正在跑的编辑器"
 *
 * 为什么底图不上色、只有聚光层上色：常驻的单色更安静，颜色只在被照亮的地方
 * 出现，既有信息量又不吵。
 *
 * 为什么用 3 列：单个 <pre> 的宽度等于最长行（11px 等宽约 700px），
 * 盖不满宽屏；3 列 + 各列错开起始行，铺满且不出现重复花纹。
 */

type Tone = "cmd" | "log" | "code" | "comment" | "sep";

type Line = { text: string; tone: Tone };

const LINES: Line[] = [
  { text: "$ poke-hook --source=cc --event=Stop", tone: "cmd" },
  { text: "[notify] session=cc-4f21 prev=running status=idle", tone: "log" },
  { text: "[gate] already_focused=false cooldown_available=true", tone: "log" },
  { text: "[popup] show edge=bottom-right ttl=manual sound=ping", tone: "log" },
  { text: "[focus] ghostty terminal_id=14f71163 match=tty", tone: "log" },
  { text: "─────", tone: "sep" },
  { text: "let session = store.upsert(payload);", tone: "code" },
  { text: "if session.status == Idle { popup.nudge() }", tone: "code" },
  { text: "match surface { Ghostty(id) => focus(id) }", tone: "code" },
  { text: "─────", tone: "sep" },
  { text: "[hook] SessionStart source=codex tty=/dev/ttys014", tone: "log" },
  { text: "[hook] PermissionRequest → pending", tone: "log" },
  { text: "[sound] play completed.mp3 vol=0.6", tone: "log" },
  { text: "─────", tone: "sep" },
  { text: "~/.pokepoke/sock            unix:// local-only", tone: "code" },
  { text: "~/.pokepoke/sessions.json   last_write=15:02:11Z", tone: "code" },
  { text: "─────", tone: "sep" },
  { text: "// never miss the moment your agent needs you", tone: "comment" },
  { text: "─────", tone: "sep" },
  { text: "$ pnpm tauri dev", tone: "cmd" },
  { text: "compiled in 2.3s · watching src/", tone: "log" },
  { text: "[ws] panel render 12 sessions · 0 dropped", tone: "log" },
  { text: "─────", tone: "sep" },
  { text: "enum SessionStatus { Pending, Running, Idle }", tone: "code" },
  { text: "fn on_event(ev: HookEvent) -> Session { ... }", tone: "code" },
  { text: "─────", tone: "sep" },
  { text: "[hook] UserPromptSubmit → running", tone: "log" },
  { text: "[reconcile] ambiguous cwd → candidate=3 resolved=1", tone: "log" },
  { text: "[herdr] pane_key=w1:p2 focus ok", tone: "log" },
  { text: "─────", tone: "sep" },
  { text: "[hook] Stop → idle (tree quiet: agents=0 queue=0)", tone: "log" },
  { text: "[popup] dismissed by user reason=click", tone: "log" },
  { text: "[focus] jumped back in 38ms", tone: "log" },
  { text: "─────", tone: "sep" },
  { text: "[drift] hook config stale → repair once", tone: "log" },
  { text: "[dedupe] same event within 100ms dropped", tone: "log" },
  { text: "─────", tone: "sep" },
  { text: "let cfg = load(~/.pokepoke/settings.json)", tone: "code" },
  { text: "if !cfg.notifications_enabled { record_only() }", tone: "code" },
  { text: "─────", tone: "sep" },
  { text: "[cmux] surface_id=3 focus ok", tone: "log" },
  { text: "[wezterm] pane=1 activated in 41ms", tone: "log" },
  { text: "[orca] pane_key=w2:p1 switch ok", tone: "log" },
  { text: "─────", tone: "sep" },
  { text: "// quiet mode silences popup and sound", tone: "comment" },
  { text: "[sound] quiet_mode active → silenced", tone: "log" },
  { text: "─────", tone: "sep" },
  { text: "$ cargo test --manifest-path src-tauri/Cargo.toml", tone: "cmd" },
  { text: "test result: ok. 899 passed; 0 failed", tone: "log" },
  { text: "─────", tone: "sep" },
  { text: "[hook] SessionEnd → session removed", tone: "log" },
  { text: "store.retain(|s| s.updated_at > cutoff);", tone: "code" },
];

// 聚光层用的语法色（暗色主题，参考 GitHub Dark）
const TONE_CLASS: Record<Tone, string> = {
  cmd: "text-[#7ee787]/85",
  log: "text-[#c9d1d9]/75",
  code: "text-[#79c0ff]/80",
  comment: "text-[#8b949e]/70",
  sep: "text-[#30363d]",
};

// 状态词单独着色：这几个词就是产品面板里那四种状态的同款语义
const STATUS_RE = /(pending|running|idle|last_failed|completed|aborted|allowed)/gi;
const STATUS_CLASS: Record<string, string> = {
  pending: "text-[#d29922]",
  running: "text-[#58a6ff]",
  idle: "text-[#3fb950]",
  last_failed: "text-[#f85149]",
  completed: "text-[#3fb950]",
  aborted: "text-[#f85149]",
  allowed: "text-[#3fb950]",
};

/** 把一行拆成「普通片段 + 状态词片段」，让状态词带自己的颜色 */
function splitStatus(text: string): { text: string; status?: string }[] {
  const parts: { text: string; status?: string }[] = [];
  let last = 0;
  for (const m of text.matchAll(STATUS_RE)) {
    const i = m.index ?? 0;
    if (i > last) parts.push({ text: text.slice(last, i) });
    parts.push({ text: m[0], status: m[0].toLowerCase() });
    last = i + m[0].length;
  }
  if (last < text.length) parts.push({ text: text.slice(last) });
  return parts;
}

const COLUMNS = 3;

export default function CodeBackdrop() {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    // 减少动效偏好：只保留静态底图，不做光标跟随
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    let queued = false;
    let x = -9999;
    let y = -9999;

    const flush = () => {
      queued = false;
      el.style.setProperty("--mx", `${x}px`);
      el.style.setProperty("--my", `${y}px`);
    };

    const onMove = (event: PointerEvent) => {
      x = event.clientX;
      y = event.clientY;
      el.style.setProperty("--spot", "1");
      if (!queued) {
        queued = true;
        requestAnimationFrame(flush);
      }
    };

    const onLeave = () => el.style.setProperty("--spot", "0");

    window.addEventListener("pointermove", onMove, { passive: true });
    window.addEventListener("pointerleave", onLeave);

    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerleave", onLeave);
    };
  }, []);

  // 各列错开起始行，避免三列看起来像复制粘贴
  const columns = Array.from({ length: COLUMNS }, (_, i) => {
    const offset = Math.floor((LINES.length / COLUMNS) * i);
    return [...LINES.slice(offset), ...LINES.slice(0, offset)];
  });

  const basePreClass =
    "h-full overflow-hidden whitespace-pre select-none font-mono text-[11px] leading-[1.55] tracking-tight";

  return (
    <div
      ref={ref}
      aria-hidden="true"
      className="pointer-events-none fixed inset-0 overflow-hidden"
      style={{
        ["--mx" as string]: "-9999px",
        ["--my" as string]: "-9999px",
        ["--spot" as string]: "0",
        maskImage:
          "linear-gradient(to bottom, transparent 0%, #000 10%, #000 82%, transparent 100%)",
        WebkitMaskImage:
          "linear-gradient(to bottom, transparent 0%, #000 10%, #000 82%, transparent 100%)",
      }}
    >
      {/* 1. 暖光：极淡的珊瑚色灯，给白色聚光一点温度 */}
      <div
        className="absolute inset-0 transition-opacity duration-500"
        style={{
          opacity: "var(--spot)",
          background:
            "radial-gradient(460px circle at var(--mx) var(--my), rgba(255,69,58,0.13) 0%, rgba(255,69,58,0.05) 42%, transparent 72%)",
        }}
      />

      {/* 2. 底图：单色、常驻 */}
      <div className="grid h-full w-full grid-cols-3 gap-10 px-8">
        {columns.map((col, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: 静态文本列，顺序不会变
          <pre key={i} className={`${basePreClass} text-white/[0.055]`}>
            {col.map((l) => `${l.text}\n`).join("")}
          </pre>
        ))}
      </div>

      {/* 3. 聚光层：按语法上色，被跟随光标的遮罩点亮 */}
      <div
        className="absolute inset-0 transition-opacity duration-500"
        style={{
          opacity: "var(--spot)",
          maskImage:
            "radial-gradient(340px circle at var(--mx) var(--my), rgba(0,0,0,1) 0%, rgba(0,0,0,0.55) 42%, transparent 74%)",
          WebkitMaskImage:
            "radial-gradient(340px circle at var(--mx) var(--my), rgba(0,0,0,1) 0%, rgba(0,0,0,0.55) 42%, transparent 74%)",
        }}
      >
        <div className="grid h-full w-full grid-cols-3 gap-10 px-8">
          {columns.map((col, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: 静态文本列，顺序不会变
            <pre key={i} className={basePreClass}>
              {col.map((l, j) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: 静态文本行，顺序不会变
                <div key={j} className={TONE_CLASS[l.tone]}>
                  {splitStatus(l.text).map((part, k) =>
                    part.status ? (
                      // biome-ignore lint/suspicious/noArrayIndexKey: 静态文本片段，顺序不会变
                      <span key={k} className={STATUS_CLASS[part.status]}>
                        {part.text}
                      </span>
                    ) : (
                      // biome-ignore lint/suspicious/noArrayIndexKey: 静态文本片段，顺序不会变
                      <span key={k}>{part.text}</span>
                    ),
                  )}
                </div>
              ))}
            </pre>
          ))}
        </div>
      </div>
    </div>
  );
}
