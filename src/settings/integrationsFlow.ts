// IntegrationsTab 异步编排状态机 —— 纯 reducer(W11,brief
// 20260813-integrations-flow-reducer)。无 React / invoke / listen 依赖。
//
// 转移语义逐条对照 IntegrationsTab.tsx 现状(2026-08-13 HEAD)抽取:
// - busy 互斥:**per-row + 全量刷新**互斥(现状按钮 disabled =
//   `refreshingAll || rowBusy[source] !== null`,不同 source 可并行,不是全表互斥);
// - 动作成功后的 rows 刷新走 `rows_loaded`(loadAll 语义,现状动作尾部都 loadAll);
// - drift 事件(现状 hook-drift-detected / hook-drift-repaired)只触发 loadAll
//   全量刷新、不打断 busy —— `drift_detected(rows)` 语义 = rows 替换;
//   `drift_repaired` 同(事件不带 source,参数可选保留契约);
// - confirm 只在 disconnect 按钮点击时打开(现状无 session 计数条件);
// - message 用 `{ kind: "i18n" | "raw" }` 区分 i18n key 与后端直返字符串
//   (现状 result.message 是后端原始文案,直接展示;其余是 t() 翻译)。

import type { IntegrationSource, ManagedIntegrationStatus } from "./integrationsLogic";

export type RowBusyKind = "connect" | "disconnect" | "repair" | null;

/** 结果提示:i18n key(渲染层 t() 翻译)或后端直返原始文案。 */
export type FlowMessage =
  | { kind: "i18n"; key: string; params?: Record<string, string | number> }
  | { kind: "raw"; text: string }
  | null;

export interface IntegrationsFlowState {
  rows: ManagedIntegrationStatus[];
  /** 首次加载中(初始 loading 态,首轮 loadAll finally 清除)。 */
  loading: boolean;
  /** 手动全量刷新中(「检查全部」按钮)。 */
  refreshingAll: boolean;
  /** 每行当前动作(connect/disconnect/repair);非 null 即该行 busy。 */
  rowBusy: Record<IntegrationSource, RowBusyKind>;
  message: FlowMessage;
  /** 断开确认弹窗的 source;null = 弹窗关闭。 */
  confirmSource: IntegrationSource | null;
}

export const INTEGRATION_SOURCES: readonly IntegrationSource[] = [
  "cc",
  "codex",
  "cursor",
  "opencode",
  "pi",
  "omp",
  "kimi",
  "grok",
  "dsh",
] as const;

export type IntegrationsFlowAction =
  /** loadAll 成功:rows 替换(动作尾部 / 事件刷新的公共路径)。 */
  | { type: "rows_loaded"; rows: ManagedIntegrationStatus[] }
  /** 首轮 loadAll 结束(finally):loading 清除。 */
  | { type: "initial_loaded" }
  /** 手动全量刷新开始:refreshingAll 置位 + 清 message。 */
  | { type: "refresh_started" }
  /** 全量刷新成功:rows 替换 + refreshingAll 清除 + check_all_done 提示。 */
  | { type: "refresh_done"; rows: ManagedIntegrationStatus[] }
  /** 全量刷新失败:refreshingAll 清除 + action_failed 提示。 */
  | { type: "refresh_failed" }
  /** 单行动作开始:该行 busy 置位 + 清 message。 */
  | {
      type: "action_started";
      source: IntegrationSource;
      kind: Exclude<RowBusyKind, null>;
    }
  /** 单行动作成功:该行 busy 清除 + 结果提示(成功无提示时 message=null)。 */
  | { type: "action_done"; source: IntegrationSource; message: FlowMessage }
  /** 单行动作失败(invoke 抛错):该行 busy 清除 + action_failed 提示。 */
  | { type: "action_failed"; source: IntegrationSource }
  /** hook-drift-detected 事件:rows 全量刷新,不打断 busy(现状语义)。 */
  | { type: "drift_detected"; rows: ManagedIntegrationStatus[] }
  /** hook-drift-repaired 事件:同 drift_detected(source 可选,事件不带)。 */
  | { type: "drift_repaired"; source?: IntegrationSource; rows?: ManagedIntegrationStatus[] }
  /** 断开确认弹窗打开。 */
  | { type: "confirm_open"; source: IntegrationSource }
  /** 断开确认弹窗关闭(取消或确认后)。 */
  | { type: "confirm_cancel" }
  /** 清结果提示。 */
  | { type: "message_clear" };

export function flowInitialState(): IntegrationsFlowState {
  const rowBusy = {} as Record<IntegrationSource, RowBusyKind>;
  for (const source of INTEGRATION_SOURCES) {
    rowBusy[source] = null;
  }
  return {
    rows: [],
    loading: true,
    refreshingAll: false,
    rowBusy,
    message: null,
    confirmSource: null,
  };
}

/** i18n 提示快捷构造。 */
export function i18nMessage(key: string, params?: Record<string, string | number>): FlowMessage {
  return { kind: "i18n", key, params };
}

/** 后端直返文案提示快捷构造。 */
export function rawMessage(text: string): FlowMessage {
  return { kind: "raw", text };
}

/** 派生:该行按钮是否可发起动作(现状 disabled = refreshingAll || 该行 busy)。 */
export function canAct(state: IntegrationsFlowState, source: IntegrationSource): boolean {
  return !state.refreshingAll && state.rowBusy[source] === null;
}

export function flowReducer(
  state: IntegrationsFlowState,
  action: IntegrationsFlowAction,
): IntegrationsFlowState {
  switch (action.type) {
    case "rows_loaded":
      return { ...state, rows: action.rows };
    case "initial_loaded":
      return { ...state, loading: false };
    case "refresh_started":
      return { ...state, refreshingAll: true, message: null };
    case "refresh_done":
      return {
        ...state,
        rows: action.rows,
        refreshingAll: false,
        message: i18nMessage("settings.integrations.check_all_done"),
      };
    case "refresh_failed":
      return {
        ...state,
        refreshingAll: false,
        message: i18nMessage("settings.integrations.action_failed"),
      };
    case "action_started":
      return {
        ...state,
        rowBusy: { ...state.rowBusy, [action.source]: action.kind },
        message: null,
      };
    case "action_done":
      return {
        ...state,
        rowBusy: { ...state.rowBusy, [action.source]: null },
        message: action.message,
      };
    case "action_failed":
      return {
        ...state,
        rowBusy: { ...state.rowBusy, [action.source]: null },
        message: i18nMessage("settings.integrations.action_failed"),
      };
    case "drift_detected":
      return { ...state, rows: action.rows };
    case "drift_repaired":
      // 现状 hook-drift-repaired 事件只触发 loadAll 刷新,不打断 busy;
      // rows 可选(事件本身不带数据,由事件处理器 refetch 后带上)。
      return action.rows ? { ...state, rows: action.rows } : state;
    case "confirm_open":
      return { ...state, confirmSource: action.source };
    case "confirm_cancel":
      return { ...state, confirmSource: null };
    case "message_clear":
      return { ...state, message: null };
  }
}
