const RELEASES_API = "https://api.github.com/repos/hellokidder/PokePoke/releases/latest";
const RELEASES_PAGE = "https://github.com/hellokidder/PokePoke/releases/latest";

/**
 * 拉取最新 release 的 .dmg 资产直链并触发下载。
 * 失败（无网络 / API 限流 / 无 dmg 资产）时退化为打开 releases 页面。
 */
export async function downloadLatestDmg(): Promise<void> {
  try {
    const res = await fetch(RELEASES_API, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) throw new Error(`GitHub API ${res.status}`);
    const data = (await res.json()) as {
      assets?: { name: string; browser_download_url: string }[];
    };
    const dmg = data.assets?.find((a) => a.name.toLowerCase().endsWith(".dmg"));
    if (!dmg) throw new Error("no dmg asset");
    window.location.href = dmg.browser_download_url;
  } catch {
    window.open(RELEASES_PAGE, "_blank", "noopener,noreferrer");
  }
}
