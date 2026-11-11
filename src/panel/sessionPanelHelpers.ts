import type { Session } from "../types";

export function projectName(session: Session): string {
  const match = session.title.match(/:\s*(.+)/);
  return match ? match[1] : session.title;
}

export function sourceLabel(session: Session): string {
  switch (session.source) {
    case "claude-code":
      return "Claude Code";
    case "cursor":
      return "Cursor";
    // codex 桌面端(ChatGPT.app,无 tty)显示 ChatGPT;CLI(有 tty)显示 Codex
    case "codex":
      return session.terminal_tty ? "Codex" : "ChatGPT";
    case "opencode":
      return "opencode";
    case "pi":
      return "Pi";
    case "omp":
      // omp 是 pi 的 fork(oh-my-pi),面板上按 CLI 名区分两个 fork
      return "omp";
    case "kimi":
      return "Kimi Code";
    case "grok":
      return "Grok";
    case "dsh":
      return "DeepSeek Harness";
    default:
      return session.source || "";
  }
}

export function workspacePath(session: Session): string {
  if (!session.workspace_path) return "";
  return session.workspace_path.replace(/^\/Users\/[^/]+/, "~");
}

export function isActive(s: Session): boolean {
  return s.status === "running" || s.status === "pending";
}

export function sortSessionsForPanel(sessions: Session[]): Session[] {
  return [...sessions].sort((a, b) => {
    const aActive = isActive(a);
    const bActive = isActive(b);
    if (aActive !== bActive) return aActive ? -1 : 1;
    return new Date(a.created_at).getTime() - new Date(b.created_at).getTime();
  });
}
