import type { SchematicOrigin } from "../ipc";

/**
 * 独立预览窗口的位置解析。
 *
 * `open_schematic_window` 用 `#/schematic-preview?…` 打开一个窗口，片段里只有
 * 实例 ID、相对路径、来源与标题。这里把片段解回结构化参数；不是预览窗口就
 * 返回 `null`，`App` 走正常界面。
 *
 * 片段来自后端，但仍按不可信输入处理：缺字段、来源非法、路径为空一律当作
 * 「不是预览窗口」，不给后端发畸形请求。
 */
export type PreviewLocation = {
  instanceId: string;
  relativePath: string;
  origin: SchematicOrigin;
  originVersion: string;
  title: string;
};

const PREVIEW_ROUTE = "#/schematic-preview";
const ORIGINS: readonly SchematicOrigin[] = ["instance", "game", "version"];

/** 解析 `location.hash`；不是合法的预览片段时返回 `null`。 */
export function parsePreviewLocation(hash: string): PreviewLocation | null {
  if (!hash.startsWith(PREVIEW_ROUTE)) {
    return null;
  }
  const queryStart = hash.indexOf("?");
  if (queryStart < 0) {
    return null;
  }
  const params = new URLSearchParams(hash.slice(queryStart + 1));

  const instanceId = (params.get("instance") ?? "").trim();
  const relativePath = (params.get("path") ?? "").trim();
  const origin = (params.get("origin") ?? "").trim();
  if (instanceId.length === 0 || relativePath.length === 0) {
    return null;
  }
  if (!ORIGINS.includes(origin as SchematicOrigin)) {
    return null;
  }

  return {
    instanceId,
    relativePath,
    origin: origin as SchematicOrigin,
    originVersion: params.get("version") ?? "",
    title: params.get("title") ?? "",
  };
}
