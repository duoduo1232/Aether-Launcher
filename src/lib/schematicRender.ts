import { colorForBlock, isAir } from "./blockColors";
import type { SchematicDetail } from "../ipc";

/**
 * 把解码后的投影数据整理成渲染器可以直接吃的体素数组。
 *
 * 纯函数、不依赖 three.js，方便在 Vitest 里验证抽样与配色逻辑：
 * - 空气方块跳过；
 * - 体素数量超过 `maxBlocks` 时按立方晶格抽样（每个轴每隔 stride 取一个），
 *   保持整体形状与比例，另用 `stride` 把方块放大回去；
 * - 坐标以整体包围盒中心为原点。
 */

export const DEFAULT_MAX_BLOCKS = 120_000;

export type RenderData = {
  /** 体素个数，也是 positions/colors 的长度除以 3。 */
  count: number;
  /** count×3，已减去包围盒中心的坐标。 */
  positions: Float32Array;
  /** count×3 的 RGB（0..1）。 */
  colors: Float32Array;
  /** 抽样步长：1 表示未抽样。 */
  stride: number;
  /** 解码数据里的实际非空气方块数（抽样前）。 */
  solidBlocks: number;
};

export function buildRenderData(
  detail: SchematicDetail,
  maxBlocks: number = DEFAULT_MAX_BLOCKS,
): RenderData {
  const solid = countSolidBlocks(detail);
  if (solid === 0) {
    return {
      count: 0,
      positions: new Float32Array(0),
      colors: new Float32Array(0),
      stride: 1,
      solidBlocks: 0,
    };
  }

  const stride =
    solid > maxBlocks ? Math.max(1, Math.ceil(Math.cbrt(solid / maxBlocks))) : 1;
  const picked: number[] = [];
  const pickedColors: string[] = [];

  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let minZ = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  let maxZ = Number.NEGATIVE_INFINITY;

  for (const region of detail.regions) {
    const [sx, sy, sz] = region.size;
    if (sx <= 0 || sy <= 0 || sz <= 0) {
      continue;
    }
    const palette = region.palette;
    const indices = region.indices;
    for (let y = 0; y < sy; y += stride) {
      for (let z = 0; z < sz; z += stride) {
        for (let x = 0; x < sx; x += stride) {
          const index = (y * sz + z) * sx + x;
          if (index >= indices.length) {
            continue;
          }
          const blockName = palette[indices[index]] ?? "";
          if (isAir(blockName)) {
            continue;
          }
          const wx = region.position[0] + x;
          const wy = region.position[1] + y;
          const wz = region.position[2] + z;
          picked.push(wx, wy, wz);
          pickedColors.push(blockName);
          if (wx < minX) minX = wx;
          if (wy < minY) minY = wy;
          if (wz < minZ) minZ = wz;
          if (wx > maxX) maxX = wx;
          if (wy > maxY) maxY = wy;
          if (wz > maxZ) maxZ = wz;
        }
      }
    }
  }

  const count = picked.length / 3;
  const centerX = count > 0 ? (minX + maxX) / 2 : 0;
  const centerY = count > 0 ? (minY + maxY) / 2 : 0;
  const centerZ = count > 0 ? (minZ + maxZ) / 2 : 0;

  const positions = new Float32Array(picked.length);
  for (let i = 0; i < picked.length; i += 3) {
    positions[i] = picked[i] - centerX;
    positions[i + 1] = picked[i + 1] - centerY;
    positions[i + 2] = picked[i + 2] - centerZ;
  }

  const colors = new Float32Array(picked.length);
  for (let i = 0; i < pickedColors.length; i += 1) {
    const [r, g, b] = hexToRgb(colorForBlock(pickedColors[i]));
    colors[i * 3] = r;
    colors[i * 3 + 1] = g;
    colors[i * 3 + 2] = b;
  }

  return {
    count,
    positions,
    colors,
    stride,
    solidBlocks: solid,
  };
}

function countSolidBlocks(detail: SchematicDetail): number {
  let total = 0;
  for (const region of detail.regions) {
    const seenAir = new Set<number>();
    region.palette.forEach((name, id) => {
      if (isAir(name)) {
        seenAir.add(id);
      }
    });
    for (const index of region.indices) {
      if (!seenAir.has(index)) {
        total += 1;
      }
    }
  }
  return total;
}

/** `#rrggbb` → [r, g, b]，范围 0..1。非法输入按白色处理。 */
export function hexToRgb(hex: string): [number, number, number] {
  const match = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!match) {
    return [1, 1, 1];
  }
  const value = Number.parseInt(match[1], 16);
  return [
    ((value >> 16) & 0xff) / 255,
    ((value >> 8) & 0xff) / 255,
    (value & 0xff) / 255,
  ];
}
