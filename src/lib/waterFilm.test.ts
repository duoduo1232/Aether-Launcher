import { describe, expect, it } from "vitest";
import { isWaterlogged, pushWaterFilm, WATER_STILL } from "./waterFilm";
import { voxelKey, type VoxelMap } from "./voxelLight";

const ATLAS = {
  [WATER_STILL]: { u0: 0.5, v0: 0.25, u1: 0.75, v1: 0.5 },
};

function solidAtOf(voxels: VoxelMap) {
  return (x: number, y: number, z: number): boolean =>
    voxels.get(voxelKey(x, y, z))?.solid === true;
}

function world(...keys: Array<[number, number, number]>): VoxelMap {
  return new Map(
    keys.map(([x, y, z]) => [
      voxelKey(x, y, z),
      {
        solid: true,
        collision: [{ min: [0, 0, 0], max: [1, 1, 1] }],
      },
    ]),
  );
}

describe("isWaterlogged", () => {
  it("waterlogged=true 的方块含水", () => {
    expect(isWaterlogged("minecraft:oak_stairs", { waterlogged: "true" })).toBe(true);
    expect(isWaterlogged("minecraft:stone_slab", { waterlogged: "true" })).toBe(true);
  });

  it("waterlogged=false 或者没有这个属性时不含水", () => {
    expect(isWaterlogged("minecraft:oak_stairs", { waterlogged: "false" })).toBe(false);
    expect(isWaterlogged("minecraft:stone_slab", {})).toBe(false);
  });

  it("海带、海草永远含水", () => {
    expect(isWaterlogged("minecraft:kelp", {})).toBe(true);
    expect(isWaterlogged("minecraft:seagrass", {})).toBe(true);
    expect(isWaterlogged("minecraft:tall_seagrass", {})).toBe(true);
  });

  it("普通方块不含水", () => {
    expect(isWaterlogged("minecraft:stone", {})).toBe(false);
  });
});

describe("pushWaterFilm", () => {
  it("周围全空时画出 5 个面（顶面 + 四面）", () => {
    const target: number[] = [];
    const added = pushWaterFilm(
      target,
      { x: 0, y: 0, z: 0, isSolid: () => false, voxels: new Map(), budget: 100 },
      ATLAS,
    );
    expect(added).toBe(5);
    // 每个面 2 个三角形 = 6 个顶点，每顶点 11 个数。
    expect(target.length).toBe(5 * 6 * 11);
  });

  it("被实心邻居挡住的面不画", () => {
    // 上方和四周全是实心方块 → 只剩底面（不画），一个面都不剩。
    const voxels = world(
      [0, 1, 0],
      [1, 0, 0],
      [-1, 0, 0],
      [0, 0, 1],
      [0, 0, -1],
    );
    const target: number[] = [];
    const added = pushWaterFilm(
      target,
      { x: 0, y: 0, z: 0, isSolid: solidAtOf(voxels), voxels, budget: 100 },
      ATLAS,
    );
    expect(added).toBe(0);
  });

  it("只有一侧被挡住时少画一个面", () => {
    const voxels = world([1, 0, 0]);
    const added = pushWaterFilm(
      [],
      { x: 0, y: 0, z: 0, isSolid: solidAtOf(voxels), voxels, budget: 100 },
      ATLAS,
    );
    expect(added).toBe(4);
  });

  it("邻居是透明方块（不是实心）时水膜照画", () => {
    // 邻居存在但 solid=false（玻璃、水），水膜要露出来。
    const voxels: VoxelMap = new Map([
      [
        voxelKey(1, 0, 0),
        {
          solid: false,
          collision: [{ min: [0, 0, 0], max: [1, 1, 1] }],
        },
      ],
    ]);
    const added = pushWaterFilm(
      [],
      { x: 0, y: 0, z: 0, isSolid: solidAtOf(voxels), voxels, budget: 100 },
      ATLAS,
    );
    expect(added).toBe(5);
  });

  it("图集里没有水面贴图时不画", () => {
    const added = pushWaterFilm(
      [],
      { x: 0, y: 0, z: 0, isSolid: () => false, voxels: new Map(), budget: 100 },
      {},
    );
    expect(added).toBe(0);
  });

  it("预算用完就停，不会超量生成", () => {
    const added = pushWaterFilm(
      [],
      { x: 0, y: 0, z: 0, isSolid: () => false, voxels: new Map(), budget: 2 },
      ATLAS,
    );
    expect(added).toBe(2);
  });

  it("UV 落在水面贴图的图集格子里", () => {
    const target: number[] = [];
    pushWaterFilm(
      target,
      { x: 0, y: 0, z: 0, isSolid: () => false, voxels: new Map(), budget: 100 },
      ATLAS,
    );
    const slot = ATLAS[WATER_STILL];
    for (let index = 0; index < target.length; index += 11) {
      const u = target[index + 6];
      const v = target[index + 7];
      expect(u).toBeGreaterThanOrEqual(slot.u0);
      expect(u).toBeLessThanOrEqual(slot.u1);
      expect(v).toBeGreaterThanOrEqual(slot.v0);
      expect(v).toBeLessThanOrEqual(slot.v1);
    }
  });

  it("水膜比整格矮：顶点最高只到 0.1", () => {
    const target: number[] = [];
    pushWaterFilm(
      target,
      { x: 0, y: 0, z: 0, isSolid: () => false, voxels: new Map(), budget: 100 },
      ATLAS,
    );
    let maxY = -Infinity;
    for (let index = 0; index < target.length; index += 11) {
      maxY = Math.max(maxY, target[index + 1]);
    }
    expect(maxY).toBeCloseTo(0.1, 6);
  });
});
