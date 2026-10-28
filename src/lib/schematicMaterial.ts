import type { AssetPack } from "../ipc";
import { loadAssetPack } from "../ipc";

/**
 * 预览用的材质来源（哪个版本的 jar）。
 *
 * 为什么存下来而不是每次问：用户装几十个版本，每次进投影预览都要在长列表里
 * 挑一遍很烦。选一次就记住，之后自动套用 —— 提取结果本身有后端缓存（按 jar
 * 的路径 + 大小 + 修改时间），所以「自动套用」不会重新解 jar。
 */

export const materialStorageKey = "aether.schematicMaterial";

/** 空串表示「纯色预览」，也即用户明确不想要材质。 */
export type MaterialPreference = string;

export function loadMaterialPreference(): MaterialPreference {
  try {
    return window.localStorage.getItem(materialStorageKey) ?? "";
  } catch {
    return "";
  }
}

export function saveMaterialPreference(version: MaterialPreference): void {
  try {
    if (version) {
      window.localStorage.setItem(materialStorageKey, version);
    } else {
      window.localStorage.removeItem(materialStorageKey);
    }
  } catch {
    // 存储不可用时，选择只在本次运行内生效。
  }
}

/**
 * 按已保存的偏好加载资源包。
 *
 * 偏好为空、或保存的版本已经不存在（玩家删了那个版本）时返回 `null`，
 * 预览退回纯色 —— 不会因为一个失效的设置让页面报错。
 */
export async function loadPreferredPack(
  preference: MaterialPreference,
): Promise<AssetPack | null> {
  if (!preference) {
    return null;
  }
  try {
    return await loadAssetPack(preference);
  } catch {
    return null;
  }
}
