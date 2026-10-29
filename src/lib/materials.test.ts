import { describe, expect, it } from "vitest";
import type { SchematicDetail } from "../ipc";
import {
  describeBoxes,
  materialsToCsv,
  stackSizeFor,
  summariseMaterials,
  toShulkers,
} from "./materials";

function detailWith(palette: string[], counts: number[]): SchematicDetail {
  return {
    instanceId: "11111111-1111-4111-8111-111111111111",
    relativePath: "a.litematic",
    fileName: "小屋.litematic",
    metadata: {
      name: "小屋",
      author: "",
      description: "",
      formatVersion: 6,
      minecraftDataVersion: 4790,
      createdForRelease: "26.1.2",
      regionCount: 1,
      totalBlocks: counts.reduce((sum, value) => sum + value, 0),
      totalVolume: counts.reduce((sum, value) => sum + value, 0),
      enclosingSize: [1, 1, counts.reduce((sum, value) => sum + value, 0)],
    },
    regions: [
      {
        name: "r",
        position: [0, 0, 0],
        size: [1, 1, counts.reduce((sum, value) => sum + value, 0)],
        palette,
        indices: counts.flatMap((count, index) => Array<number>(count).fill(index)),
      },
    ],
  };
}

describe("stackSizeFor", () => {
  it("treats ordinary building blocks as 64", () => {
    expect(stackSizeFor("minecraft:cherry_planks")).toBe(64);
    expect(stackSizeFor("minecraft:stone")).toBe(64);
    expect(stackSizeFor("minecraft:glass")).toBe(64);
  });

  it("treats signs and ender pearls as 16", () => {
    expect(stackSizeFor("minecraft:oak_sign")).toBe(16);
    expect(stackSizeFor("minecraft:ender_pearl")).toBe(16);
    expect(stackSizeFor("minecraft:armor_stand")).toBe(16);
  });

  it("treats tools and anvils as unstackable", () => {
    expect(stackSizeFor("minecraft:anvil")).toBe(1);
    expect(stackSizeFor("minecraft:diamond_pickaxe")).toBe(1);
    expect(stackSizeFor("minecraft:oak_boat")).toBe(1);
  });
});

describe("toShulkers", () => {
  it("converts 64-stack items into shulkers, stacks and loose", () => {
    // 一个潜影盒 27×64 = 1728。1800 = 1 盒 + 1 组(64) + 8。
    expect(toShulkers(1800, 64)).toEqual({ shulkers: 1, stacks: 1, loose: 8 });
  });

  it("converts 16-stack items with the smaller shulker capacity", () => {
    // 一个潜影盒 27×16 = 432。450 = 1 盒 + 1 组(16) + 2。
    expect(toShulkers(450, 16)).toEqual({ shulkers: 1, stacks: 1, loose: 2 });
  });

  it("treats unstackable items as one per slot", () => {
    // 27 格 = 27 个一盒。30 = 1 盒 + 3 个。
    expect(toShulkers(30, 1)).toEqual({ shulkers: 1, stacks: 0, loose: 3 });
  });
});

describe("summariseMaterials", () => {
  it("adds up blocks and skips air", () => {
    const rows = summariseMaterials(
      detailWith(["minecraft:air", "minecraft:stone", "minecraft:cherry_planks"], [10, 5, 3]),
    );
    expect(rows.map((row) => [row.label, row.count])).toEqual([
      ["石头", 5],
      ["樱花木板", 3],
    ]);
  });

  it("merges different block states of the same block", () => {
    const rows = summariseMaterials(
      detailWith(
        [
          "minecraft:oak_stairs[facing=north,shape=straight]",
          "minecraft:oak_stairs[facing=south,shape=inner_left]",
        ],
        [2, 3],
      ),
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].count).toBe(5);
  });

  it("sorts the most-used material first", () => {
    const rows = summariseMaterials(
      detailWith(["minecraft:dirt", "minecraft:stone"], [3, 9]),
    );
    expect(rows[0].label).toBe("石头");
  });
});

describe("describeBoxes", () => {
  it("joins the non-zero parts", () => {
    expect(describeBoxes({ shulkers: 2, stacks: 3, loose: 4 })).toBe("2 盒 + 3 组 + 4 个");
    expect(describeBoxes({ shulkers: 0, stacks: 1, loose: 0 })).toBe("1 组");
    expect(describeBoxes({ shulkers: 0, stacks: 0, loose: 0 })).toBe("0");
  });
});

describe("materialsToCsv", () => {
  it("writes a header, the file name and one row per material", () => {
    const rows = summariseMaterials(detailWith(["minecraft:stone"], [70]));
    const csv = materialsToCsv("小屋.litematic", rows);
    const lines = csv.split("\r\n");
    expect(lines[0]).toBe("投影,小屋.litematic");
    expect(lines[2]).toBe("材料,总数,一组数量,潜影盒,组,零散,换算");
    expect(lines[3]).toBe("石头,70,64,0,1,6,1 组 + 6 个");
  });
});
