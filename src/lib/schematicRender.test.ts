import { describe, expect, it } from "vitest";
import type { SchematicDetail } from "../ipc";
import { buildRenderData, hexToRgb } from "./schematicRender";

function detailWith(
  size: [number, number, number],
  palette: string[],
  indices: number[],
): SchematicDetail {
  return {
    instanceId: "test-instance",
    fileName: "test.litematic",
    metadata: {
      name: "test",
      author: "",
      description: "",
      formatVersion: 6,
      minecraftDataVersion: 3955,
      createdForRelease: "1.21.1",
      regionCount: 1,
      totalBlocks: indices.length,
      totalVolume: indices.length,
      enclosingSize: size,
    },
    regions: [
      {
        name: "main",
        position: [0, 0, 0],
        size,
        palette,
        indices,
      },
    ],
  };
}

describe("buildRenderData", () => {
  it("skips air blocks and maps colors", () => {
    // 2×1×1：一个是石头一个是空气。
    const data = buildRenderData(
      detailWith([2, 1, 1], ["minecraft:air", "minecraft:stone"], [0, 1]),
    );
    expect(data.solidBlocks).toBe(1);
    expect(data.count).toBe(1);
    expect(data.stride).toBe(1);
    // 石头的代表色是灰色 #7d7d7d（颜色经过 Float32，按近似值比较）。
    const [r, g, b] = Array.from(data.colors);
    expect(r).toBeCloseTo(0x7d / 255, 6);
    expect(g).toBeCloseTo(0x7d / 255, 6);
    expect(b).toBeCloseTo(0x7d / 255, 6);
    // 只有一个体素时坐标在原点。
    expect(Array.from(data.positions)).toEqual([0, 0, 0]);
  });

  it("centers voxels on the bounding box", () => {
    // 3×1×1，全是石头：中心 x=1，体素坐标应为 -1、0、1。
    const data = buildRenderData(
      detailWith(
        [3, 1, 1],
        ["minecraft:stone"],
        [0, 0, 0],
      ),
    );
    expect(data.count).toBe(3);
    const xs = [data.positions[0], data.positions[3], data.positions[6]].sort(
      (a, b) => a - b,
    );
    expect(xs).toEqual([-1, 0, 1]);
  });

  it("samples on a lattice when above the block budget", () => {
    // 100×100×1 全石头 = 10000 块；预算 2000 → stride = ceil(cbrt(5)) = 2。
    const size: [number, number, number] = [100, 100, 1];
    const indices = new Array<number>(10000).fill(0);
    const data = buildRenderData(detailWith(size, ["minecraft:stone"], indices), 2000);
    expect(data.stride).toBe(2);
    // 抽样后约 50×50 = 2500 个（含边缘完整行）。
    expect(data.count).toBe(2500);
  });

  it("handles an all-air schematic", () => {
    const data = buildRenderData(
      detailWith([2, 2, 2], ["minecraft:air"], new Array(8).fill(0)),
    );
    expect(data.count).toBe(0);
    expect(data.solidBlocks).toBe(0);
  });
});

describe("hexToRgb", () => {
  it("converts known colors and falls back to white", () => {
    expect(hexToRgb("#ff0000")).toEqual([1, 0, 0]);
    expect(hexToRgb("#7d7d7d")).toEqual([0x7d / 255, 0x7d / 255, 0x7d / 255]);
    expect(hexToRgb("not-a-color")).toEqual([1, 1, 1]);
  });
});
