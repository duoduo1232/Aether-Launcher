import type { AssetPack, AssetQuad, AssetVariant, SchematicDetail } from "../ipc";
import { isAir } from "./blockColors";

/**
 * 把「投影里的方块」和「从 jar 提取的方块模型」合成可直接喂给 Three 的几何。
 *
 * 和纯色渲染的区别：
 * - 每个方块不再是一个整立方体，而是模型里定义的**若干盒子**（楼梯 15 个面、
 *   栅栏更少），所以半砖、楼梯、栏杆的形状是对的；
 * - 每个面带图集 UV，看到的是真实的木板纹理而不是一块纯色；
 * - 带 `cullface` 的面在邻居占住该方向时整面跳过 —— 一栋房子的内墙面不会
 *   白白绘制；
 * - 按 `opaque` / `cutout` / `translucent` 分三批，透明方块才能正确混合。
 *
 * 全部是纯函数，不依赖 Three，方便单测。
 */

/** 三个渲染层各一份几何数据。 */
export type LayeredGeometry = {
  opaque: LayerBuffers;
  cutout: LayerBuffers;
  translucent: LayerBuffers;
  /** 没有资源包、或方块不在资源包里时，用来回退成纯色的方块位置与颜色。 */
  fallback: {
    positions: Float32Array;
    colors: Float32Array;
    count: number;
    stride: number;
  };
  /** 实际画出的四边形数，用于界面提示。 */
  quadCount: number;
  /** 有多少方块因为资源包里没有它而回退成纯色。 */
  missingBlocks: number;
};

/** 一个渲染层的顶点缓冲。 */
export type LayerBuffers = {
  positions: Float32Array;
  normals: Float32Array;
  uvs: Float32Array;
  /** 每个顶点三份（三角形索引会展开）。 */
  count: number;
};

const EMPTY_LAYER: LayerBuffers = {
  positions: new Float32Array(0),
  normals: new Float32Array(0),
  uvs: new Float32Array(0),
  count: 0,
};

/** 一个方块实例在网格化时需要的信息。 */
type Voxel = {
  x: number;
  y: number;
  z: number;
  /** 方块 id，如 `minecraft:cherry_planks`。 */
  block: string;
  /** 状态字符串里的属性（如 `facing=north,half=top`），用来挑 variant。 */
  state: Record<string, string>;
};

/**
 * 构建几何。
 *
 * `maxQuads` 是安全上限：一座几万方块的建筑可能产生上百万个四边形，
 * 超过就退回纯色渲染，避免把页面卡死。
 */
export function buildTexturedGeometry(
  detail: SchematicDetail,
  pack: AssetPack | null,
  maxQuads = 400_000,
): LayeredGeometry {
  if (!pack) {
    return {
      opaque: EMPTY_LAYER,
      cutout: EMPTY_LAYER,
      translucent: EMPTY_LAYER,
      fallback: emptyFallback(),
      quadCount: 0,
      missingBlocks: 0,
    };
  }

  // 先把体素收集起来，同时建一张坐标 → 方块 的表用于邻居查询（面剔除）。
  const voxels = collectVoxels(detail);
  const occupied = new Set<string>();
  for (const voxel of voxels) {
    occupied.add(key(voxel.x, voxel.y, voxel.z));
  }

  const opaque: number[] = [];
  const cutout: number[] = [];
  const translucent: number[] = [];
  const fallbackPositions: number[] = [];
  const fallbackColors: number[] = [];
  let quadCount = 0;
  let missingBlocks = 0;

  // 按方块名缓存「这次用了哪个 variant」，同一个方块状态只挑一次。
  const variantCache = new Map<string, AssetVariant | null>();

  for (const voxel of voxels) {
    const stateKey = `${voxel.block}|${stateSignature(voxel.state)}`;
    let variant = variantCache.get(stateKey);
    if (variant === undefined) {
      variant = pickVariant(pack, voxel.block, voxel.state);
      variantCache.set(stateKey, variant);
    }
    if (!variant) {
      missingBlocks += 1;
      fallbackPositions.push(voxel.x, voxel.y, voxel.z);
      continue;
    }

    for (const quad of variant.model.quads) {
      if (quad.cullface && isCulled(occupied, voxel, quad.cullface)) {
        continue;
      }
      // 超过上限就停：继续画只会把页面拖死，前面已画的部分仍然可看。
      if (quadCount >= maxQuads) {
        continue;
      }
      quadCount += 1;
      const target =
        quad.layer === "translucent"
          ? translucent
          : quad.layer === "cutout"
            ? cutout
            : opaque;
      pushQuad(target, voxel, variant, quad);
    }
  }

  return {
    opaque: toLayer(opaque),
    cutout: toLayer(cutout),
    translucent: toLayer(translucent),
    fallback: {
      positions: new Float32Array(fallbackPositions),
      colors: new Float32Array(fallbackColors),
      count: fallbackPositions.length / 3,
      stride: 1,
    },
    quadCount,
    missingBlocks,
  };
}

/** 收集投影里的所有非空气方块。 */
function collectVoxels(detail: SchematicDetail): Voxel[] {
  const voxels: Voxel[] = [];
  for (const region of detail.regions) {
    const [sx, sy, sz] = region.size;
    if (sx <= 0 || sy <= 0 || sz <= 0) {
      continue;
    }
    const palette = region.palette;
    const indices = region.indices;
    for (let y = 0; y < sy; y += 1) {
      for (let z = 0; z < sz; z += 1) {
        for (let x = 0; x < sx; x += 1) {
          const index = (y * sz + z) * sx + x;
          if (index >= indices.length) {
            continue;
          }
          const entry = palette[indices[index]] ?? "";
          if (!entry || isAir(entry)) {
            continue;
          }
          const { block, state } = parsePaletteEntry(entry);
          voxels.push({
            x: region.position[0] + x,
            y: region.position[1] + y,
            z: region.position[2] + z,
            block,
            state,
          });
        }
      }
    }
  }
  return voxels;
}

/**
 * 调色板条目可能是 `minecraft:oak_stairs[facing=north,half=top]` 这种带属性的
 * 形式（Litematica 会把方块状态写进去），拆成 id + 属性表。
 */
export function parsePaletteEntry(entry: string): {
  block: string;
  state: Record<string, string>;
} {
  const bracket = entry.indexOf("[");
  if (bracket < 0) {
    return { block: entry.trim(), state: {} };
  }
  const block = entry.slice(0, bracket).trim();
  const inner = entry.slice(bracket + 1, entry.lastIndexOf("]"));
  const state: Record<string, string> = {};
  for (const pair of inner.split(",")) {
    const [name, value] = pair.split("=");
    if (name && value) {
      state[name.trim()] = value.trim();
    }
  }
  return { block, state };
}

/**
 * 从方块的 variant 列表里挑一个匹配当前属性的。
 *
 * 优先选「属性完全匹配」的；都不完全匹配时，选匹配项最多的那个 —— 比随便
 * 取第一个更接近真实形状（比如 `facing=north` 的楼梯方向是对的）。
 */
export function pickVariant(
  pack: AssetPack,
  block: string,
  state: Record<string, string>,
): AssetVariant | null {
  const variants = pack.blocks[block];
  if (!variants || variants.length === 0) {
    return null;
  }
  let best: AssetVariant | null = null;
  let bestScore = -1;
  for (const variant of variants) {
    let score = 0;
    let ok = true;
    for (const [name, value] of variant.when) {
      if (state[name] !== value) {
        ok = false;
        break;
      }
      score += 1;
    }
    if (!ok) {
      continue;
    }
    // 属性全匹配且条目数最多者优先；`when` 为空表示「总是适用」，作为兜底。
    if (score > bestScore) {
      best = variant;
      bestScore = score;
    } else if (score === bestScore && best && variant.when.length === 0) {
      best = variant;
    }
  }
  return best ?? variants[0] ?? null;
}

/** 邻居占住 `face` 方向时，这个面可以整面跳过。 */
function isCulled(
  occupied: Set<string>,
  voxel: Voxel,
  face: string,
): boolean {
  const [dx, dy, dz] = FACE_OFFSET[face] ?? [0, 0, 0];
  return occupied.has(key(voxel.x + dx, voxel.y + dy, voxel.z + dz));
}

const FACE_OFFSET: Record<string, [number, number, number]> = {
  down: [0, -1, 0],
  up: [0, 1, 0],
  north: [0, 0, -1],
  south: [0, 0, 1],
  west: [-1, 0, 0],
  east: [1, 0, 0],
};

function key(x: number, y: number, z: number): string {
  return `${x},${y},${z}`;
}

function stateSignature(state: Record<string, string>): string {
  return Object.keys(state)
    .sort()
    .map((name) => `${name}=${state[name]}`)
    .join(",");
}

/**
 * 把一个四边形推进缓冲，套上 variant 的旋转并平移到体素位置。
 *
 * 三角形拆分用固定的 (0,1,2) (0,2,3) 顺序，与 `face_corners` 的逆时针一致。
 */
function pushQuad(
  target: number[],
  voxel: Voxel,
  variant: AssetVariant,
  quad: AssetQuad,
): void {
  const rotated = quad.positions.map((position) =>
    rotate(position, variant.rotationX, variant.rotationY),
  );
  const normals = rotate(quad.normal, variant.rotationX, variant.rotationY, true);

  for (const index of [0, 1, 2, 0, 2, 3]) {
    const position = rotated[index];
    target.push(
      voxel.x + position[0],
      voxel.y + position[1],
      voxel.z + position[2],
    );
    target.push(normals[0], normals[1], normals[2]);
    target.push(quad.uvs[index][0], quad.uvs[index][1]);
  }
}

/**
 * 按 Minecraft 的规则旋转：先绕 X（0/90/180/270），再绕 Y。
 *
 * 旋转是绕方块中心的，坐标在 0..1 空间里，所以先平移到 -0.5..0.5。
 */
function rotate(
  value: [number, number, number] | readonly number[],
  rotationX: number,
  rotationY: number,
  isNormal = false,
): [number, number, number] {
  let [x, y, z] = [value[0], value[1], value[2]];
  if (!isNormal) {
    x -= 0.5;
    y -= 0.5;
    z -= 0.5;
  }

  const radiansX = (rotationX * Math.PI) / 180;
  if (radiansX !== 0) {
    const cos = Math.round(Math.cos(radiansX));
    const sin = Math.round(Math.sin(radiansX));
    const ny = y * cos - z * sin;
    const nz = y * sin + z * cos;
    y = ny;
    z = nz;
  }

  const radiansY = (rotationY * Math.PI) / 180;
  if (radiansY !== 0) {
    const cos = Math.round(Math.cos(radiansY));
    const sin = Math.round(Math.sin(radiansY));
    const nx = x * cos + z * sin;
    const nz = -x * sin + z * cos;
    x = nx;
    z = nz;
  }

  if (!isNormal) {
    x += 0.5;
    y += 0.5;
    z += 0.5;
  }
  return [x, y, z];
}

/** `[px,py,pz, nx,ny,nz, u,v] × 6` → 三份独立缓冲。 */
function toLayer(values: number[]): LayerBuffers {
  const count = values.length / 8;
  const positions = new Float32Array(count * 3);
  const normals = new Float32Array(count * 3);
  const uvs = new Float32Array(count * 2);
  for (let index = 0; index < count; index += 1) {
    const base = index * 8;
    positions[index * 3] = values[base];
    positions[index * 3 + 1] = values[base + 1];
    positions[index * 3 + 2] = values[base + 2];
    normals[index * 3] = values[base + 3];
    normals[index * 3 + 1] = values[base + 4];
    normals[index * 3 + 2] = values[base + 5];
    uvs[index * 2] = values[base + 6];
    uvs[index * 2 + 1] = values[base + 7];
  }
  return { positions, normals, uvs, count };
}

function emptyFallback() {
  return {
    positions: new Float32Array(0),
    colors: new Float32Array(0),
    count: 0,
    stride: 1,
  };
}
