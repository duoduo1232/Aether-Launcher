import { callCommand } from "./call";

/**
 * 方块资源包（从版本 jar 提取的贴图与模型）。
 *
 * 为什么需要它：只给每个方块一种纯色，木板看不到纹理、半砖看不到斜面、玻璃
 * 看不到透明，整座建筑会糊成一片马赛克。真正的观感要用 Minecraft 自己的
 * blockstate / model / 贴图 —— 它们都在玩家已经下载好的版本 jar 里。
 *
 * 提取是**用户主动触发**的：界面提示「选择版本提取」，用户点了才解 jar。
 */
declare module "./call" {
  interface CommandArgs {
    list_asset_pack_candidates: void;
    load_asset_pack: { version: string };
    asset_pack_status: void;
  }

  interface CommandResults {
    list_asset_pack_candidates: AssetPackCandidate[];
    load_asset_pack: AssetPack;
    asset_pack_status: boolean;
  }
}

/** 可以提取资源包的候选版本。 */
export type AssetPackCandidate = {
  version: string;
  /** jar 大小（MB），用来提示这是完整客户端。 */
  sizeMb: number;
  /** 是否已经提取过。 */
  extracted: boolean;
};

/** 一个面：四个顶点 + 图集 UV + 法线。 */
export type AssetQuad = {
  /** 归一化到 0..1 方块空间的四个顶点。 */
  positions: [number, number, number][];
  uvs: [number, number][];
  normal: [number, number, number];
  /** 邻居占住这个方向时可以整面跳过；`null` 表示始终绘制。 */
  cullface: string | null;
  layer: "opaque" | "cutout" | "translucent";
};

/** 一个方块状态绑定。 */
export type AssetVariant = {
  when: [string, string][];
  rotationX: number;
  rotationY: number;
  model: {
    quads: AssetQuad[];
    textureIds: string[];
    fullCube: boolean;
  };
};

/** 图集里一张贴图的位置（0..1 UV）。 */
export type AtlasSlot = {
  u0: number;
  v0: number;
  u1: number;
  v1: number;
};

/** 提取结果。 */
export type AssetPack = {
  sourceVersion: string;
  /** 方块 id → 状态绑定列表。 */
  blocks: Record<string, AssetVariant[]>;
  atlas: Record<string, AtlasSlot>;
  atlasSize: number;
  /** 图集的 PNG（data URL），直接交给 Three 的纹理加载器。 */
  atlasDataUrl: string;
};

/**
 * 扫描本机可提取资源包的版本。
 *
 * 只列目录、读文件大小，不解压任何内容。
 */
export async function listAssetPackCandidates(): Promise<AssetPackCandidate[]> {
  return callCommand("list_asset_pack_candidates");
}

/** 从指定版本提取（或读缓存）资源包。`version` 必须来自上面的列表。 */
export async function loadAssetPack(version: string): Promise<AssetPack> {
  return callCommand("load_asset_pack", { version });
}

/** 是否已经提取过任一资源包。 */
export async function assetPackStatus(): Promise<boolean> {
  return callCommand("asset_pack_status");
}
