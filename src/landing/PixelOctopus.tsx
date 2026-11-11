/**
 * PixelOctopus — 产品 icon 的 1:1 像素画 SVG 组件(landing 视觉签名)。
 *
 * 矩阵来源:`src-tauri/icons/tray-default.png`(44×44,2×2 像素块)逐块中心
 * 采样出的 22×22 原生像素矩阵。提取方法(icon 改版时可重跑):
 *   ffmpeg -v error -i src-tauri/icons/tray-default.png -f rawvideo -pix_fmt rgba /tmp/tray.raw
 *   然后按 2×2 块中心采样,映射下方 5 色调色板。
 * 色板与 icon 完全一致:A 主体紫 / B 阴影紫(触手)/ C 瞳孔 / D 眼白 / E 高光。
 * shape-rendering: crispEdges 保证放大后像素边缘锐利,无抗锯齿模糊。
 */

const PALETTE: Record<string, string> = {
  A: "#8D88FF",
  B: "#665FD8",
  C: "#1D1840",
  D: "#E1DFFF",
  E: "#FFFFFF",
};

const MATRIX: string[] = [
  "......................",
  "......................",
  "..........AB..........",
  ".........AAAB.........",
  "........AAAAAB........",
  ".......AAAAAAAB.......",
  "......AAAAAAAAAB......",
  "......AAAAAAAAAB......",
  ".....AAAAAAAAAAAB.....",
  "....AAAEEAAAAEEAAB....",
  "....AAAECAAAACEAAB....",
  "...AAAAAAAAAAAAAAAB...",
  "...BAADDAAAAAAADDAB...",
  "....BAAACAAACAAAAB....",
  ".....BAAACCCAAAAB.....",
  "......AAAAAAAAAB......",
  "......AABAABAABB......",
  "......BB.BB.BB.BB.....",
  ".....BB..BB.BB..BB....",
  "......B...B..B..B.....",
  "......................",
  "......................",
];

/** 同色相连续段合并成一条 rect 串,减少 SVG 节点数 */
function buildRects(): { fill: string; d: string }[] {
  const byColor = new Map<string, string[]>();
  MATRIX.forEach((row, y) => {
    let x = 0;
    while (x < row.length) {
      const ch = row[x];
      if (ch === ".") {
        x += 1;
        continue;
      }
      let run = 1;
      while (x + run < row.length && row[x + run] === ch) run += 1;
      const list = byColor.get(ch) ?? [];
      list.push(`M${x} ${y}h${run}v1h${-run}z`);
      byColor.set(ch, list);
      x += run;
    }
  });
  return [...byColor.entries()].map(([ch, d]) => ({ fill: PALETTE[ch], d: d.join("") }));
}

const RECTS = buildRects();

export default function PixelOctopus({
  size = 160,
  className = "",
}: {
  size?: number;
  className?: string;
}) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 22 22"
      shapeRendering="crispEdges"
      role="img"
      aria-label="PokePoke pixel octopus"
      className={className}
    >
      {RECTS.map((r) => (
        <path key={r.fill} fill={r.fill} d={r.d} />
      ))}
    </svg>
  );
}
