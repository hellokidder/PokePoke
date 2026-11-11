import confetti from "canvas-confetti";

/**
 * Realistic Look(散开铺屏版)—— 基于 canvas-confetti 官方「真实感」预设。
 * 官方预设是单点抛洒,视觉偏集中;这里改为横向多点发射(左/中/右三炮),
 * 并加大扩散与初速,让彩带铺满整屏宽度、更散漫,同时保留真实礼花的层次感
 * (每个发射点都叠加 5 组不同初速/扩散度/粒子数)。
 * 参考: https://www.kirilv.com/canvas-confetti/ → "Realistic Look"
 *
 * 返回一个在动画结束后 resolve 的 Promise —— 全屏覆盖层据此自毁关窗。
 * resolve 时机必须晚于粒子彻底落地/淡出,否则会连同还在飘的粒子一起关窗,
 * 表现为「动效没结束就突然消失」。
 * fire-and-forget,内部自建全屏 canvas,放完自动清理。
 */
export function fireCelebration(): Promise<void> {
  // 尊重「减少动效」偏好,避免给晕动症用户造成不适
  if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) {
    return Promise.resolve();
  }

  // 关键:默认的 `confetti()` 导出内部用 `useWorker: true`,靠
  // `new Worker(URL.createObjectURL(new Blob(...)))` 建一个 blob: worker。
  // 生产构建的 CSP(script-src 'self',无 worker-src / blob:)会拦掉它,
  // 而 WKWebView 下这次拦截不会同步抛错——库拿到一个「死」worker,把画布
  // transferControlToOffscreen 转移过去后再没人绘制,表现为「窗开了但零粒子」。
  // dev 走 Vite dev server 不注入该 CSP,所以只在正式构建里复现。
  // 解法:自建全屏 canvas 并显式 useWorker:false,走主线程绘制,绕开 blob worker。
  const canvas = document.createElement("canvas");
  canvas.style.position = "fixed";
  canvas.style.inset = "0";
  canvas.style.width = "100%";
  canvas.style.height = "100%";
  canvas.style.pointerEvents = "none";
  document.body.appendChild(canvas);

  const instance = confetti.create(canvas, {
    useWorker: false,
    resize: true,
  });

  // 三个横向发射点,铺满屏幕宽度;y 偏下,粒子向上炸开后受重力回落
  const origins = [
    { x: 0.2, y: 0.75 },
    { x: 0.5, y: 0.8 },
    { x: 0.8, y: 0.75 },
  ];
  const countPerOrigin = 120;

  const fire = (
    origin: { x: number; y: number },
    particleRatio: number,
    opts: confetti.Options,
  ) => {
    void instance({
      origin,
      disableForReducedMotion: true,
      ...opts,
      particleCount: Math.floor(countPerOrigin * particleRatio),
    });
  };

  for (const origin of origins) {
    // 5 组叠加:窄而快的主炮 + 逐渐变宽变慢的外围,堆出真实礼花的层次。
    // 相比官方预设整体调宽 spread、提高 startVelocity,让粒子飞得更开、更满屏。
    fire(origin, 0.25, { spread: 50, startVelocity: 65 });
    fire(origin, 0.2, { spread: 90 });
    fire(origin, 0.35, { spread: 140, decay: 0.91, scalar: 0.8 });
    fire(origin, 0.1, { spread: 150, startVelocity: 35, decay: 0.92, scalar: 1.2 });
    fire(origin, 0.1, { spread: 150, startVelocity: 55 });
  }

  // 粒子全部落地/淡出约需 ~4s(低速粒子受重力慢慢飘落);
  // 留足时间再 resolve,避免提前关窗截断动效。resolve 前清掉 canvas。
  return new Promise<void>((resolve) => {
    window.setTimeout(() => {
      instance.reset();
      canvas.remove();
      resolve();
    }, 4200);
  });
}
