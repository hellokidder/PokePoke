import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { fireCelebration } from "../lib/confetti";

/**
 * 全屏彩带覆盖层窗口的内容(window label = "confetti")。
 * 这个窗口由 Rust 端 `celebrate` 命令创建:透明、置顶、点击穿透、铺满整屏。
 * 挂载即放彩带,放完主动 invoke 关掉自己。
 */
export default function ConfettiOverlay() {
  useEffect(() => {
    let closed = false;
    const close = () => {
      if (closed) return;
      closed = true;
      void invoke("close_confetti_overlay").catch(() => {});
    };

    fireCelebration().then(close);

    // 双保险:即便 fireCelebration 因故不 resolve,兜底定时器也会关掉覆盖层。
    // 取值需 > fireCelebration 正常时长(~4.2s),否则会提前关窗截断彩带。
    const fallback = window.setTimeout(close, 5500);
    return () => window.clearTimeout(fallback);
  }, []);

  return null;
}
