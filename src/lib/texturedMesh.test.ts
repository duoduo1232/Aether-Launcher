import { describe, expect, it } from "vitest";
import {
  buildTexturedGeometry,
  parsePaletteEntry,
  pickVariant,
} from "./texturedMesh";
import type {
  AssetPack,
  AssetQuad,
  AssetVariant,
  SchematicDetail,
} from "../ipc";

/**
 * 贴图网格化。
 *
 * 这些测试钉的是「不会再退回纯色方块」的关键路径：属性解析、variant 匹配、
 * 面剔除、以及按渲染层分批。
 */

/** 一个占满整格的方块：六个面各一个四边形，都带 cullface。 */
function fullCubeQuad(face: string): AssetQuad {
  return {
    positions: [
      [0, 0, 0],
      [1, 0, 0],
      [1, 1, 0],
      [0, 1, 0],
    ],
    uvs: [
      [0, 1],
      [1, 1],
      [1, 0],
      [0, 0],
    ],
    normal: [0, 0, -1],
    cullface: face,
    layer: "opaque",
  };
}

const FACES = ["down", "up", "north", "south", "west", "east"];

function cubeVariant(block: string): AssetVariant {
  return {
    when: [],
    rotationX: 0,
    rotationY: 0,
    model: {
      quads: FACES.map(fullCubeQuad),
      textureIds: [`minecraft:block/${block}`],
      fullCube: true,
    },
  };
}

function packWith(blocks: Record<string, AssetVariant[]>): AssetPack {
  return {
    sourceVersion: "test",
    blocks,
    atlas: {},
    atlasSize: 16,
    atlasDataUrl: "data:image/png;base64,",
  };
}

function detailWith(
  palette: string[],
  size: [number, number, number] = [1, 1, 1],
): SchematicDetail {
  return {
    instanceId: "11111111-1111-4111-8111-111111111111",
    relativePath: "a.litematic",
    fileName: "a.litematic",
    metadata: {
      name: "a",
      author: "",
      description: "",
      formatVersion: 6,
      minecraftDataVersion: 3955,
      createdForRelease: "1.21.1",
      regionCount: 1,
      totalBlocks: 1,
      totalVolume: 1,
      enclosingSize: size,
    },
    regions: [
      {
        name: "r",
        position: [0, 0, 0],
        size,
        palette,
        indices: Array.from({ length: size[0] * size[1] * size[2] }, (_, i) => i % palette.length),
      },
    ],
  };
}

describe("parsePaletteEntry", () => {
  it("splits the block id from its state", () => {
    expect(
      parsePaletteEntry("minecraft:oak_stairs[facing=north,half=top]"),
    ).toEqual({
      block: "minecraft:oak_stairs",
      state: { facing: "north", half: "top" },
    });
  });

  it("keeps a bare id unchanged", () => {
    expect(parsePaletteEntry("minecraft:stone")).toEqual({
      block: "minecraft:stone",
      state: {},
    });
  });
});

describe("pickVariant", () => {
  it("prefers the variant whose properties all match", () => {
    const pack = packWith({
      "minecraft:oak_stairs": [
        { ...cubeVariant("oak_planks"), when: [["facing", "north"]], rotationY: 0 },
        { ...cubeVariant("oak_planks"), when: [["facing", "east"]], rotationY: 90 },
      ],
    });
    const picked = pickVariant(pack, "minecraft:oak_stairs", { facing: "east" });
    expect(picked?.rotationY).toBe(90);
  });

  it("falls back to a catch-all variant", () => {
    const pack = packWith({
      "minecraft:stone": [{ ...cubeVariant("stone"), when: [] }],
    });
    expect(pickVariant(pack, "minecraft:stone", { anything: "x" })).not.toBeNull();
  });

  it("returns null for a block the pack does not have", () => {
    expect(pickVariant(packWith({}), "minecraft:modded_block", {})).toBeNull();
  });
});

describe("buildTexturedGeometry", () => {
  it("culls the shared faces of two adjacent blocks", () => {
    // 两个相邻的整方块：中间那对面互相挡住，应该被剔除。
    // 单方块 6 面 → 两个孤立方块 12 面；相邻时 12 - 2 = 10 面。
    const pack = packWith({ "minecraft:stone": [cubeVariant("stone")] });
    const single = buildTexturedGeometry(
      detailWith(["minecraft:stone"], [1, 1, 1]),
      pack,
    );
    expect(single.quadCount).toBe(6);

    const adjacent = buildTexturedGeometry(
      detailWith(["minecraft:stone"], [2, 1, 1]),
      pack,
    );
    expect(adjacent.quadCount).toBe(10);
  });

  it("produces triangles for every quad, not points", () => {
    const pack = packWith({ "minecraft:stone": [cubeVariant("stone")] });
    const geometry = buildTexturedGeometry(
      detailWith(["minecraft:stone"], [1, 1, 1]),
      pack,
    );
    // 6 个四边形 × 6 个顶点（两个三角形）。
    expect(geometry.opaque.count).toBe(36);
    expect(geometry.opaque.positions.length).toBe(36 * 3);
    expect(geometry.opaque.uvs.length).toBe(36 * 2);
    expect(geometry.opaque.normals.length).toBe(36 * 3);
  });

  it("counts blocks missing from the pack instead of dropping them", () => {
    const pack = packWith({ "minecraft:stone": [cubeVariant("stone")] });
    const geometry = buildTexturedGeometry(
      detailWith(["minecraft:stone", "minecraft:modded_thing"], [2, 1, 1]),
      pack,
    );
    expect(geometry.missingBlocks).toBe(1);
    expect(geometry.fallback.count).toBe(1);
  });

  it("returns an empty geometry when there is no pack", () => {
    const geometry = buildTexturedGeometry(
      detailWith(["minecraft:stone"]),
      null,
    );
    expect(geometry.quadCount).toBe(0);
    expect(geometry.opaque.count).toBe(0);
  });

  it("stops at the quad budget", () => {
    const pack = packWith({ "minecraft:stone": [cubeVariant("stone")] });
    const geometry = buildTexturedGeometry(
      detailWith(["minecraft:stone"], [4, 1, 1]),
      pack,
      5,
    );
    expect(geometry.quadCount).toBe(5);
  });

  it("routes translucent faces to their own layer", () => {
    const glass = cubeVariant("glass");
    glass.model.quads = glass.model.quads.map((quad) => ({
      ...quad,
      layer: "translucent" as const,
    }));
    const pack = packWith({ "minecraft:glass": [glass] });
    const geometry = buildTexturedGeometry(
      detailWith(["minecraft:glass"], [1, 1, 1]),
      pack,
    );
    expect(geometry.translucent.count).toBe(36);
    expect(geometry.opaque.count).toBe(0);
  });
});
