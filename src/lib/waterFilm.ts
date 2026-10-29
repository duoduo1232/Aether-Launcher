import type { AtlasSlot, AssetPack } from "../ipc";
import { aoShades, voxelKey, type FaceName, type VoxelMap } from "./voxelLight";

/**
 * 含水方块的水膜几何。
 *
 * 楼梯、台阶、栅栏这些 `waterlogged=true` 的方块，在原版里除了自己的模型，
 * 内部还会叠一层流体。不画的话它们的「含水」状态看起来和干的完全一样 ——
 * 投影里的水下楼梯会和普通楼梯没区别。
 *
 * 水位照 shulkr 的流体规格（`Ft()`）：含水方块的水面在方块底部 `0.1` 格
 * （`D = r.waterlogged ? .1 : .05`，单位是格），侧面从底面铺到这条水位线。
 */

/** 水面贴图的图集键。后端 `build_asset_pack` 会确保它进图集。 */
export const WATER_STILL = "minecraft:block/water_still";

/** 水面色，`#3F76E4`。和 Rust 侧 `WATER_TINT` 是同一个值。 */
const WATER_TINT: [number, number, number] = [0.247, 0.463, 0.894];

/** 水膜高度（格）。 */
const FILM_HEIGHT = 0.1;

/**
 * 永远含水的方块：blockstate 里没有 `waterlogged` 属性，但它们总在水里。
 *
 * 照 shulkr 的 `wt` 集合。
 */
const ALWAYS_WATERLOGGED = new Set([
  "kelp",
  "kelp_plant",
  "seagrass",
  "tall_seagrass",
]);

/**
 * 这个方块内部是否含水。
 *
 * 判据照 shulkr 的 `Tt()`：`waterlogged === 'true'`，或者本身就是水生植物。
 */
export function isWaterlogged(
  block: string,
  state: Record<string, string>,
): boolean {
  if (state.waterlogged === "true") {
    return true;
  }
  const name = block.startsWith("minecraft:")
    ? block.slice("minecraft:".length)
    : block;
  return ALWAYS_WATERLOGGED.has(name);
}

type FilmFace = {
  face: FaceName;
  /** 四个角，方块内局部坐标（0..1），顺序与 pushQuad 一致。 */
  corners: Array<[number, number, number]>;
  normal: [number, number, number];
  /** 这个面朝外的邻居偏移，用来判断能不能省掉。 */
  offset: [number, number, number];
};

/**
 * 五个面：顶面 + 四个侧面。底面贴着地面，看不见，不画。
 */
const FILM_FACES: FilmFace[] = [
  {
    face: "up",
    corners: [
      [0, FILM_HEIGHT, 1],
      [1, FILM_HEIGHT, 1],
      [1, FILM_HEIGHT, 0],
      [0, FILM_HEIGHT, 0],
    ],
    normal: [0, 1, 0],
    offset: [0, 1, 0],
  },
  {
    face: "north",
    corners: [
      [0, FILM_HEIGHT, 0],
      [1, FILM_HEIGHT, 0],
      [1, 0, 0],
      [0, 0, 0],
    ],
    normal: [0, 0, -1],
    offset: [0, 0, -1],
  },
  {
    face: "south",
    corners: [
      [1, FILM_HEIGHT, 1],
      [0, FILM_HEIGHT, 1],
      [0, 0, 1],
      [1, 0, 1],
    ],
    normal: [0, 0, 1],
    offset: [0, 0, 1],
  },
  {
    face: "west",
    corners: [
      [0, FILM_HEIGHT, 1],
      [0, FILM_HEIGHT, 0],
      [0, 0, 0],
      [0, 0, 1],
    ],
    normal: [-1, 0, 0],
    offset: [-1, 0, 0],
  },
  {
    face: "east",
    corners: [
      [1, FILM_HEIGHT, 0],
      [1, FILM_HEIGHT, 1],
      [1, 0, 1],
      [1, 0, 0],
    ],
    normal: [1, 0, 0],
    offset: [1, 0, 0],
  },
];

export type WaterFilmOptions = {
  /** 方块所在格的世界坐标。 */
  x: number;
  y: number;
  z: number;
  /** 这一格是否挡光（实心满方块）。水膜被实心邻居挡住时不用画。 */
  isSolid: (x: number, y: number, z: number) => boolean;
  /** 周围有哪些格，用来区分「缺格」和「透明格」。 */
  voxels: VoxelMap;
  /** 还能再加多少个四边形。 */
  budget: number;
};

/**
 * 生成一个含水方块的水膜，推进目标缓冲，返回新增的四边形数。
 *
 * 侧面只在邻居不是完整实心方块时才画 —— 四面都被墙包住的水膜外面也看不见。
 */
export function pushWaterFilm(
  target: number[],
  options: WaterFilmOptions,
  atlas: AssetPack["atlas"],
): number {
  const slot = atlas[WATER_STILL];
  if (slot === undefined) {
    return 0;
  }
  const { x, y, z, isSolid, voxels, budget } = options;
  let added = 0;
  for (const item of FILM_FACES) {
    if (added >= budget) {
      break;
    }
    const [dx, dy, dz] = item.offset;
    const nx = x + dx;
    const ny = y + dy;
    const nz = z + dz;
    // 邻居是实心方块 → 这面水膜被挡住。只在邻居**存在**时才判：
    // 缺格（场景边缘）说明外面是空气，水膜要露出来。
    if (voxels.has(voxelKey(nx, ny, nz)) && isSolid(nx, ny, nz)) {
      continue;
    }
    const shades = aoShades(item.face, item.corners, x, y, z, isSolid);
    pushFilmQuad(target, x, y, z, item, shades, slot);
    added += 1;
  }
  return added;
}

/**
 * 推一个水膜四边形。
 *
 * 顶点顺序与 `texturedMesh.pushQuad` 一致（左上、左下、右下、右上），三角形
 * 拆分同样按 AO 决定对角线。UV 落在水面贴图的图集格子里，铺满整格 ——
 * 水面贴图本来就是 16×16 平铺的，一格对准一格即可。
 */
function pushFilmQuad(
  target: number[],
  x: number,
  y: number,
  z: number,
  item: FilmFace,
  shades: [number, number, number, number],
  slot: AtlasSlot,
): void {
  // 面内 (u, v) → 图集坐标。和 `build_quad` 的口径一致：v=0 是贴图顶部。
  const uvs: Array<[number, number]> = [
    [slot.u0, slot.v0],
    [slot.u0, slot.v1],
    [slot.u1, slot.v1],
    [slot.u1, slot.v0],
  ];
  // AO 对角线翻转：避免亮度沿对角线出现折痕。
  const flip = shades[0] + shades[2] < shades[1] + shades[3];
  const order = flip ? [1, 2, 3, 1, 3, 0] : [0, 1, 2, 0, 2, 3];
  for (const index of order) {
    const corner = item.corners[index];
    target.push(x + corner[0], y + corner[1], z + corner[2]);
    target.push(item.normal[0], item.normal[1], item.normal[2]);
    target.push(uvs[index][0], uvs[index][1]);
    const shade = shades[index] ?? 1;
    target.push(
      WATER_TINT[0] * shade,
      WATER_TINT[1] * shade,
      WATER_TINT[2] * shade,
    );
  }
}
