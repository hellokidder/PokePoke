export type SessionStatus = "pending" | "running" | "idle" | "last_failed";
export interface Session {
  id: string;
  task_id: string;
  external_session_id: string | null;
  title: string;
  message: string;
  source: string | null;
  priority: "normal" | "high";
  status: SessionStatus;
  created_at: string;
  updated_at: string;
  terminal_tty: string | null;
  workspace_path: string | null;
  failure_reason: string | null;
  /** Ghostty terminal surface stable UUID; iTerm2 / Cursor 通常为 null */
  terminal_id: string | null;
  /** terminal_id 写入时 Ghostty tab 的 cwd,用于 L1 绑定校验 */
  terminal_id_captured_cwd: string | null;
  /** cc/codex hook payload 携带的 transcript jsonl 路径 */
  transcript_path: string | null;
}
