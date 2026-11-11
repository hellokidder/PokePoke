import { cn } from "../lib/utils";
import type { SessionStatus } from "../types";

const SOURCE_COLORS: Record<string, string> = {
  "claude-code": "#D97757",
  cc: "#D97757",
  cursor: "#00D4AA",
  codex: "#10A37F",
  opencode: "#F97316",
  pi: "#8B5CF6",
  omp: "#22D3EE",
  kimi: "#5B8DEF",
  grok: "#FFFFFF",
  dsh: "#4D6BFE",
  build: "#4CAF50",
  make: "#4CAF50",
  deploy: "#4FC3F7",
  test: "#9C6ADE",
  cli: "#43A047",
};

const DEFAULT_COLOR = "#88AAFF";

/** Hash a string to a vibrant HSL color (fixed saturation/lightness for dark bg).
 *  - **FNV-1a 32-bit + MurmurHash3 finalizer**:FNV-1a 单独对短 / 长度相近的
 *    字符串(workspace_path 通常长度类似)输出会有 collision(实测
 *    `tuxedo workspace` 和 `cursor-22bc6f4e` 都得到 hue=15)。补一个
 *    Murmur3 finalizer mix 大幅提升 avalanche,单 bit 输入差异 → 多 bit
 *    输出差异,collision 降到接近随机水平。
 *  - **黄金角 137.5° 步进**:相邻 hash 值经它倍乘后 hue 仍相距 ~137°。
 *  - **避开屎黄区** (hue 40~70):这段 hue 配 65% sat / 60% light 视觉上是
 *    暗橙偏黄,在深色背景下很丑。落到这区的 hue 平移到对面色相(+180°),
 *    让整套色板都"养眼"。
 *  - sat / light 略提到 70/62,让所有色都更鲜艳明亮。 */
export function hashColor(seed: string): string {
  // FNV-1a 32-bit
  let hash = 0x811c9dc5 | 0;
  for (let i = 0; i < seed.length; i++) {
    hash ^= seed.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  // MurmurHash3 32-bit finalizer:大幅提升 avalanche
  hash ^= hash >>> 16;
  hash = Math.imul(hash, 0x85ebca6b);
  hash ^= hash >>> 13;
  hash = Math.imul(hash, 0xc2b2ae35);
  hash ^= hash >>> 16;

  const u32 = hash >>> 0;
  let hue = (u32 * 137.50776405003785) % 360;
  // 避开 40~70 的屎黄/暗橙区,平移到对面色相
  if (hue >= 40 && hue < 70) {
    hue = (hue + 180) % 360;
  }
  return `hsl(${hue}, 70%, 62%)`;
}

/** Darken a color for shadow edges */
function darkenColor(color: string): string {
  if (color.startsWith("hsl(")) {
    return color.replace(/,\s*([\d.]+)%\)$/, (_, l) => `, ${Math.max(0, Number(l) - 18)}%)`);
  }
  if (/^#[0-9a-f]{6}$/i.test(color)) {
    const r = parseInt(color.slice(1, 3), 16);
    const g = parseInt(color.slice(3, 5), 16);
    const b = parseInt(color.slice(5, 7), 16);
    const f = 0.72;
    return `rgb(${Math.round(r * f)},${Math.round(g * f)},${Math.round(b * f)})`;
  }
  return color;
}

/** Blush tint: 40% opacity version of the body color */
function blushColor(color: string): string {
  if (color.startsWith("hsl(")) {
    return color.replace("hsl(", "hsla(").replace(")", ", 0.4)");
  }
  return `${color}66`;
}

// C1-a shape: 18×18 teardrop octopus (水滴弹 body, 4 tentacles, no highlights/ornaments)
// Each entry: [startX, endX] inclusive, for each row y=0..17
const SHAPE_ROWS: [number, number][][] = [
  [[8, 9]], // 0
  [[7, 10]], // 1
  [[6, 11]], // 2
  [[5, 12]], // 3
  [[4, 13]], // 4
  [[4, 13]], // 5
  [[3, 14]], // 6
  [[2, 15]], // 7
  [[2, 15]], // 8
  [[1, 16]], // 9
  [[1, 16]], // 10
  [[2, 15]], // 11
  [[3, 14]], // 12
  [[4, 13]], // 13
  [[4, 13]], // 14
  [
    [4, 5],
    [7, 8],
    [10, 11],
    [13, 14],
  ], // 15 – tentacles top
  [
    [3, 4],
    [7, 8],
    [10, 11],
    [14, 15],
  ], // 16 – tentacles mid
  [
    [4, 4],
    [8, 8],
    [11, 11],
    [14, 14],
  ], // 17 – tentacle tips
];

// Precompute body cell set and split into normal vs shadow cells.
// Shadow: right edge (!hasRight) | bottom edge (!hasBottom) | tentacle left edge (y≥15 && !hasLeft)
function buildShapeCells() {
  const body = new Set<string>();
  SHAPE_ROWS.forEach((segs, y) => {
    segs.forEach(([s, e]) => {
      for (let x = s; x <= e; x++) body.add(`${x},${y}`);
    });
  });

  const normal: [number, number][] = [];
  const shadow: [number, number][] = [];
  body.forEach((k) => {
    const [x, y] = k.split(",").map(Number);
    const isShadow =
      !body.has(`${x + 1},${y}`) ||
      !body.has(`${x},${y + 1}`) ||
      (y >= 15 && !body.has(`${x - 1},${y}`));
    (isShadow ? shadow : normal).push([x, y]);
  });

  return { normal, shadow };
}

const { normal: BODY_NORMAL, shadow: BODY_SHADOW } = buildShapeCells();

// Cheeks: two pixels each side at row 10 (widest area)
const CHEEK_CELLS: [number, number][] = [
  [4, 10],
  [5, 10],
  [13, 10],
  [14, 10],
];

type Pixel = [number, number, string];

// Eyes sit at left=[5,7], right=[11,7] (top-left corner of 2×2 eye area)
// Mouth center at cx=8, cy=11
export function getExpression(status: SessionStatus): Pixel[] {
  const W = "#FFFFFF";
  const D = "#1a1a2e";

  switch (status) {
    case "pending":
      return [
        // Open eyes: 2×2 white, pupils facing inward
        [5, 7, W],
        [6, 7, W],
        [5, 8, W],
        [6, 8, D], // left eye, pupil right
        [11, 7, W],
        [12, 7, W],
        [11, 8, D],
        [12, 8, W], // right eye, pupil left
        // Smile: corners up, center dips
        [6, 11, D],
        [7, 12, D],
        [8, 12, D],
        [9, 12, D],
        [10, 11, D],
      ];
    case "running":
      return [
        // Half-lid squint: ink on lower row only (top row stays body color)
        [5, 8, D],
        [6, 8, D],
        [11, 8, D],
        [12, 8, D],
        // Smile
        [6, 11, D],
        [7, 12, D],
        [8, 12, D],
        [9, 12, D],
        [10, 11, D],
      ];
    case "idle":
      return [
        // ^^ happy eyes：上排像素着色，弯弯眯眯的。"空闲但在线"的视觉语义
        [5, 7, D],
        [6, 7, D],
        [11, 7, D],
        [12, 7, D],
        // Smile
        [6, 11, D],
        [7, 12, D],
        [8, 12, D],
        [9, 12, D],
        [10, 11, D],
      ];
    case "last_failed":
      return [
        // >< 闭眼：每只眼取 2×2 的对角两个像素，像斜线
        [5, 7, D],
        [6, 8, D],
        [12, 7, D],
        [11, 8, D],
        // 嘴角下垂：中间高两边低，倒 V 形
        [6, 12, D],
        [7, 11, D],
        [8, 11, D],
        [9, 11, D],
        [10, 12, D],
      ];
  }
}

function PixelMonster({ color, status }: { color: string; status: SessionStatus }) {
  const shadowCol = darkenColor(color);
  const blush = blushColor(color);
  const expression = getExpression(status);

  return (
    <svg
      viewBox="0 0 18 18"
      className="w-full h-full"
      shapeRendering="crispEdges"
      aria-hidden="true"
    >
      {/* Body fill */}
      {BODY_NORMAL.map(([x, y]) => (
        <rect key={`b${x},${y}`} x={x} y={y} width={1} height={1} fill={color} />
      ))}
      {/* Shadow edges */}
      {BODY_SHADOW.map(([x, y]) => (
        <rect key={`s${x},${y}`} x={x} y={y} width={1} height={1} fill={shadowCol} />
      ))}
      {/* Cheek blush */}
      {CHEEK_CELLS.map(([x, y]) => (
        <rect key={`c${x},${y}`} x={x} y={y} width={1} height={1} fill={blush} />
      ))}
      {/* Expression (eyes + mouth) */}
      {expression.map(([x, y, fill]) => (
        <rect key={`e${x},${y},${fill}`} x={x} y={y} width={1} height={1} fill={fill} />
      ))}
    </svg>
  );
}

interface SourceIconProps {
  source: string | null;
  status?: SessionStatus;
  animate?: boolean;
  /** Unique seed (e.g. task_id) to generate a distinct color per session */
  colorSeed?: string;
}

export default function SourceIcon({
  source,
  status = "pending",
  animate = true,
  colorSeed,
}: SourceIconProps) {
  const key = source?.toLowerCase().replace(/[\s_]/g, "-") ?? "";
  const color = colorSeed ? hashColor(colorSeed) : SOURCE_COLORS[key] || DEFAULT_COLOR;
  const animClass = animate && status === "running" ? "animate-monster-sway" : "";

  return (
    <div
      className={cn(
        "w-9 h-9 shrink-0 rounded-[10px] overflow-hidden flex items-center justify-center",
        animClass,
      )}
    >
      <PixelMonster color={color} status={status} />
    </div>
  );
}
