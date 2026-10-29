import { describe, expect, it } from "vitest";
import {
  buildTexturedGeometry,
  parsePaletteEntry,
  pickVariants,
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

/**
 * 一个占满整格的方块：六个面各一个四边形，都带 cullface。
 *
 * **每个面必须是真实的位置**，不能六个面共用同一组顶点。剔除要按旋转后的
 * 世界方向查邻居，位置全一样时「转没转」算出来的邻居碰巧相同，测试就抓不住
 * 方向没转的 bug。
 */
function fullCubeQuad(face: string): AssetQuad {
  const positions = CUBE_FACES[face];
  return {
    positions,
    uvs: [
      [0, 1],
      [1, 1],
      [1, 0],
      [0, 0],
    ],
    normal: FACE_NORMALS[face],
    cullface: face,
    layer: "opaque",
    uv: ["minecraft:block/test", "minecraft:block/test"],
  };
}

/**
 * 立方体六个面的顶点（顺序照 shulkr 的 `ft()`），以及各自的法线。
 *
 * 顶点顺序要和 `face_corners` 一致，否则法线方向和绕序对不上，旋转后的
 * 世界方向推算会错。
 */
const CUBE_FACES: Record<string, [number, number, number][]> = {
  down: [
    [0, 0, 0],
    [1, 0, 0],
    [1, 0, 1],
    [0, 0, 1],
  ],
  up: [
    [0, 1, 1],
    [1, 1, 1],
    [1, 1, 0],
    [0, 1, 0],
  ],
  north: [
    [0, 0, 0],
    [0, 1, 0],
    [1, 1, 0],
    [1, 0, 0],
  ],
  south: [
    [1, 0, 1],
    [1, 1, 1],
    [0, 1, 1],
    [0, 0, 1],
  ],
  west: [
    [0, 0, 1],
    [0, 1, 1],
    [0, 1, 0],
    [0, 0, 0],
  ],
  east: [
    [1, 0, 0],
    [1, 1, 0],
    [1, 1, 1],
    [1, 0, 1],
  ],
};

const FACE_NORMALS: Record<string, [number, number, number]> = {
  down: [0, -1, 0],
  up: [0, 1, 0],
  north: [0, 0, -1],
  south: [0, 0, 1],
  west: [-1, 0, 0],
  east: [1, 0, 0],
};
const FACES = Object.keys(CUBE_FACES);


function cubeVariant(block: string): AssetVariant {
  return {
    when: [],
    // 测试里的变体都按 `multipart` 处理（可叠加）。
    exclusive: false,
    rotationX: 0,
    rotationY: 0,
    model: {
      quads: FACES.map(fullCubeQuad),
      textureIds: [`minecraft:block/${block}`],
      fullCube: true,
    },
  };
}

/**
 * 一个带透明像素的整方块（玻璃、树叶这种）。
 *
 * 真实数据里它们的模型同样是 `fullCube`，但贴图带 alpha，所以面落在
 * `cutout` 层 —— 判据就是这一层，不是方块名。
 */
function transparentCubeVariant(block: string): AssetVariant {
  const variant = cubeVariant(block);
  return {
    ...variant,
    model: {
      ...variant.model,
      quads: variant.model.quads.map((quad) => ({ ...quad, layer: "cutout" as const })),
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

/**
 * 旋转矩阵必须与 Minecraft 一致。
 *
 * 方向搞反的话几何和法线会一起翻向：面朝里、贴图镜像，画面上就是「材质和
 * 方块对不上」。这里用一个不对称的四边形钉住旋转结果。
 */
describe("variant rotation", () => {
  /** 只画南面（+Z）的一个半高盒子，便于观察旋转。 */
  function southOnly(): AssetVariant {
    return {
      when: [],
      exclusive: false,
      rotationX: 0,
      rotationY: 0,
      model: {
        quads: [
          {
            positions: [
              [0, 0, 1],
              [0, 1, 1],
              [1, 1, 1],
              [1, 0, 1],
            ],
            uvs: [
              [0, 0],
              [0, 1],
              [1, 1],
              [1, 0],
            ],
            normal: [0, 0, 1],
            cullface: null,
            layer: "opaque",
            uv: ["minecraft:block/test", "minecraft:block/test"],
          },
        ],
        textureIds: ["minecraft:block/test"],
        fullCube: false,
      },
    };
  }

  function firstVertex(rotationY: number): [number, number, number] {
    const variant = { ...southOnly(), rotationY };
    const geometry = buildTexturedGeometry(
      detailWith(["minecraft:test"]),
      packWith({ "minecraft:test": [variant] }),
    );
    return [
      geometry.opaque.positions[0],
      geometry.opaque.positions[1],
      geometry.opaque.positions[2],
    ];
  }

  it("绕 Y 旋转 90° 符合标准右手系", () => {
    // 点 (0,0,1) 相对方块中心是 (0,0,+0.5)，标准 Y 旋转 90° 后到 (-0.5,0,0)，
    // 也就是绝对坐标 (0,0,0)。
    const [x, y, z] = firstVertex(90);
    expect(x).toBeCloseTo(0, 5);
    expect(y).toBeCloseTo(0, 5);
    expect(z).toBeCloseTo(0, 5);
  });

  it("绕 Y 旋转 180° 把 +Z 面转到 -Z", () => {
    const [x, y, z] = firstVertex(180);
    expect(x).toBeCloseTo(1, 5);
    expect(y).toBeCloseTo(0, 5);
    expect(z).toBeCloseTo(0, 5);
  });

  it("没有旋转时顶点原样保留", () => {
    const [x, y, z] = firstVertex(0);
    expect([x, y, z]).toEqual([0, 0, 1]);
  });
});

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

describe("pickVariants", () => {
  it("returns every part a multipart block needs at once", () => {
    const pack = packWith({
      "minecraft:oak_stairs": [
        { ...cubeVariant("oak_planks"), when: [["facing", "east"]], rotationY: 90 },
        { ...cubeVariant("oak_planks"), when: [["shape", "straight"]], rotationY: 0 },
        { ...cubeVariant("oak_planks"), when: [["shape", "inner_left"]], rotationY: 180 },
      ],
    });
    // 直梯朝东：方向和形状两个部件都该画出来，转角那个不画。
    const picked = pickVariants(pack, "minecraft:oak_stairs", {
      facing: "east",
      shape: "straight",
    });
    expect(picked.map((variant) => variant.rotationY)).toEqual([90, 0]);
  });

  it("keeps a catch-all part, which multipart uses as a fallback", () => {
    const pack = packWith({
      "minecraft:stone": [{ ...cubeVariant("stone"), when: [] }],
    });
    expect(pickVariants(pack, "minecraft:stone", { anything: "x" })).toHaveLength(1);
  });

  it("returns nothing for a block the pack does not have", () => {
    expect(pickVariants(packWith({}), "minecraft:modded_block", {})).toEqual([]);
  });
});

describe("buildTexturedGeometry", () => {
  it("culls the shared faces of two adjacent blocks", () => {
    // 同种方块相邻 → 中间那对面完全重合，剔除：12 - 2 = 10。
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

  it("culls against the world direction a rotated quad actually faces", () => {
    // 90° 旋转只是把六个方向做了一次排列，所以**整方块**（六个方向各一个面）
    // 无论查模型方向还是世界方向，被剔掉的面数都一样 —— 拿整方块写测试是抓不
    // 住这个 bug 的，我试过，两种算法都是 10。
    //
    // 能区分的只有「面不全的模型」：这里用一个 5 面的半砖形状（缺 down），
    // variant 带 `x:90`。这时模型的 up 面转完朝世界 south，north 面朝世界 up。
    //
    // 北边有一个实心邻居：
    // - 查世界方向 → north 面转到了 up，北边没有邻居，up 面不该被剔 → 5 面；
    // - 查模型方向 → 拿 north 去查北边邻居，剔掉 → 只剩 4 面。
    const slabQuads = ["up", "north", "south", "west", "east"];
    const base = cubeVariant("oak_slab");
    const variant: AssetVariant = {
      ...base,
      rotationX: 90,
      exclusive: true,
      model: {
        ...base.model,
        quads: base.model.quads.filter((quad) =>
          slabQuads.includes(quad.cullface ?? ""),
        ),
        fullCube: false,
      },
    };
    const pack = packWith({
      "minecraft:oak_slab": [variant],
      "minecraft:stone": [cubeVariant("stone")],
    });

    // 调色板顺序决定 indices[i] = i % 2：size [1,1,2] 时 z=0 是半砖、z=1 是石头。
    // 北边（z=0 外侧，z=-1）没有方块，北边的剔除不该发生。
    const geometry = buildTexturedGeometry(
      detailWith(["minecraft:oak_slab", "minecraft:stone"], [1, 1, 2]),
      pack,
    );
    // 半砖 5 面全保留；石头 6 面，南面外侧没有邻居也全保留。
    expect(geometry.quadCount).toBe(11);
  });

  it("culls a rotated block whose real neighbour is on the world side", () => {
    // 同一个 5 面半砖，`x:90`，这次把实心邻居放在**下边**。
    // 旋转后模型的 down 面朝世界 west、up 面朝世界 south、north 面朝世界 up。
    // 下边有邻居时，被剔的应该是转到 down 的那一面（模型 south）→ 4 面。
    const slabQuads = ["up", "north", "south", "west", "east"];
    const base = cubeVariant("oak_slab");
    const variant: AssetVariant = {
      ...base,
      rotationX: 90,
      exclusive: true,
      model: {
        ...base.model,
        quads: base.model.quads.filter((quad) =>
          slabQuads.includes(quad.cullface ?? ""),
        ),
        fullCube: false,
      },
    };
    const pack = packWith({
      "minecraft:oak_slab": [variant],
      "minecraft:stone": [cubeVariant("stone")],
    });
    // size [1,2,1]：y=0 是石头（下），y=1 是半砖（上）。
    const geometry = buildTexturedGeometry(
      detailWith(["minecraft:stone", "minecraft:oak_slab"], [1, 2, 1]),
      pack,
    );
    // 石头 6 面全保留（上面被半砖盖住，但半砖不是整方块不挡）；
    // 半砖模型里的 south 面转到世界 down，下边是石头 → 剔掉 → 4 面。合计 10。
    expect(geometry.quadCount).toBe(10);
  });


  it("does not cull stairs of the same id but different orientation", () => {
    // 楼梯不是整方块，接触面形状不一定一致，按 id 剔除会开洞。
    const stairs = cubeVariant("oak_stairs");
    const pack = packWith({
      "minecraft:oak_stairs": [
        { ...stairs, model: { ...stairs.model, fullCube: false } },
      ],
    });
    const geometry = buildTexturedGeometry(
      detailWith(["minecraft:oak_stairs"], [2, 1, 1]),
      pack,
    );
    // 非整方块不互剔：两根各 6 面。
    expect(geometry.quadCount).toBe(12);
  });

  it("culls the shared faces between two water blocks", () => {
    // 水挨着水，中间那对面不该画 —— 否则一块水塘叠出几百层重合面，
    // 视角一动就闪。
    const pack = packWith({ "minecraft:water": [cubeVariant("water")] });
    const geometry = buildTexturedGeometry(
      detailWith(["minecraft:water"], [2, 1, 1]),
      pack,
    );
    expect(geometry.quadCount).toBe(10);
  });

  it("keeps a stone wall intact next to glass and leaves", () => {
    // 判据分两档：实心满方块（石头）挡住任何邻居；非实心的（玻璃、树叶）
    // 只挡同名方块。所以：
    // - 石头 6 面一面不少（错判成"被玻璃挡住"就会在墙上挖洞）；
    // - 玻璃朝石头那一面**确实看不见**，剔掉 1 面；
    // - 玻璃朝树叶、树叶朝玻璃都照画（不同名且都不是实心）。
    // 合计 6 + 5 + 6 = 17。
    const pack = packWith({
      "minecraft:stone": [cubeVariant("stone")],
      "minecraft:glass": [transparentCubeVariant("glass")],
      "minecraft:cherry_leaves": [transparentCubeVariant("cherry_leaves")],
    });
    const geometry = buildTexturedGeometry(
      detailWith(
        ["minecraft:stone", "minecraft:glass", "minecraft:cherry_leaves"],
        [3, 1, 1],
      ),
      pack,
    );
    expect(geometry.quadCount).toBe(17);
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

  it("counts blocks missing from the pack instead of drawing a cube", () => {
    const pack = packWith({ "minecraft:stone": [cubeVariant("stone")] });
    const geometry = buildTexturedGeometry(
      detailWith(["minecraft:stone", "minecraft:modded_thing"], [2, 1, 1]),
      pack,
    );
    // 找不到模型的方块不画：退回纯色立方体会和真实模型差半格，看起来是错位。
    expect(geometry.missingBlocks).toBe(1);
    // 只有一个石头时六个面都在；modded 那个不画。
    expect(geometry.quadCount).toBeGreaterThan(0);
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
