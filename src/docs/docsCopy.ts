import type { Locale } from "../i18n/strings";
import { docsCopyEn } from "./docsCopyEn";
import { docsCopyZh } from "./docsCopyZh";

/**
 * /docs 文案结构。内容事实源:
 * - README.md(安装 / 接入 / 设置 / 卸载 / 隐私)
 * - src-tauri/src/herdr_remote.rs 头注释(herdr 远程模式,权威)
 * 漂移时以上述源头为准回改,不要在组件里硬编码文案。
 */

export type DocsBlock =
  | { kind: "para"; text: string }
  | { kind: "list"; items: string[] }
  | { kind: "code"; lines: string[] }
  | { kind: "table"; head: string[]; rows: string[][] }
  | { kind: "note"; text: string };

export type DocsSection = {
  /** 锚点 id,如 "herdr-remote" */
  id: string;
  /** mono 编号,如 "4" */
  index: string;
  title: string;
  blocks: DocsBlock[];
};

export type DocsCopy = {
  header: {
    manual: string;
    tagline: string;
    backHome: string;
    download: string;
  };
  sections: DocsSection[];
};

export function getDocsCopy(locale: Locale): DocsCopy {
  return locale === "en" ? docsCopyEn : docsCopyZh;
}
