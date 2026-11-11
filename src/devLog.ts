import { invoke } from "@tauri-apps/api/core";

// 前端统一日志通道：把 webview 的 console 信息汇聚到 ~/.pokepoke/pokepoke.log
// （和后端、hook 的日志统一文件，方便排障时一并打包）。
// dev_mode 由后端 logger 控制——前端只管"发"，后端按级别决定写不写。
//
// 详见 docs/archive/briefs/20260420-dev-mode-logging.md / docs/dev-logging.md

type DevLogLevel = "debug" | "info" | "warn" | "error";

export function devLog(level: DevLogLevel, tag: string, message: string): void {
  // 仍然往 console 打一份，方便开发时直接看 webview devtools
  if (level === "warn") {
    console.warn(`[${tag}]`, message);
  } else if (level === "error") {
    console.error(`[${tag}]`, message);
  } else {
    console.log(`[${tag}]`, message);
  }
  // 静默吃掉异常——日志通道不应反过来阻塞业务
  invoke("log_from_frontend", { level, tag, message }).catch(() => {});
}

export const devDebug = (tag: string, message: string) => devLog("debug", tag, message);
export const devInfo = (tag: string, message: string) => devLog("info", tag, message);
export const devWarn = (tag: string, message: string) => devLog("warn", tag, message);
export const devError = (tag: string, message: string) => devLog("error", tag, message);
