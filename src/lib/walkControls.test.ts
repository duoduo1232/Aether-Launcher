import * as THREE from "three";
import { describe, expect, it } from "vitest";
import {
  collides,
  moveAxis,
  PLAYER,
  solidQuery,
  spawnPoint,
} from "./walkControls";
import { voxelKey, type VoxelMap } from "./voxelLight";

/** 造一个「挡光也挡人」的整格方块。 */
function solidAt(
  x: number,
  y: number,
  z: number,
): [string, { solid: boolean; collision: Array<{ min: [number, number, number]; max: [number, number, number] }> }] {
  return [
    voxelKey(x, y, z),
    {
      solid: true,
      collision: [{ min: [0, 0, 0], max: [1, 1, 1] }],
    },
  ];
}

/** 造一个「不挡光但挡人」的格子（楼梯、台阶、栅栏）。 */
function volumeAt(
  x: number,
  y: number,
  z: number,
): [string, { solid: boolean; collision: Array<{ min: [number, number, number]; max: [number, number, number] }> }] {
  return [
    voxelKey(x, y, z),
    {
      solid: false,
      collision: [{ min: [0, 0, 0], max: [1, 1, 1] }],
    },
  ];
}

/** 造一个「只有 1/16 高」的薄片（地毯、压力板）。 */
function carpetAt(
  x: number,
  y: number,
  z: number,
): [string, { solid: boolean; collision: Array<{ min: [number, number, number]; max: [number, number, number] }> }] {
  return [
    voxelKey(x, y, z),
    {
      solid: false,
      collision: [{ min: [0, 0, 0], max: [1, 1 / 16, 1] }],
    },
  ];
}

type Entry = ReturnType<typeof solidAt>;

function world(...entries: Entry[]): VoxelMap {
  return new Map(entries);
}

const bounds = {
  minX: 0,
  minY: 0,
  minZ: 0,
  maxX: 4,
  maxY: 4,
  maxZ: 4,
};

describe("solidQuery", () => {
  it("看 collides，不看 solid", () => {
    const query = solidQuery(world(solidAt(1, 0, 1)));
    expect(query(1, 0, 1)).toBe(true);
    expect(query(1, 0, 2)).toBe(false);
  });

  it("地毯这种薄片只有 1/16 高，站着不该被挡住", () => {
    // 地毯的碰撞盒只到 0.0625，玩家的脚在 0.0625 以上时不该算撞。
    const voxels = world(carpetAt(1, 0, 1));
    // 站在地毯上面（脚 y=1，也就是下一格：地毯本身在 y=0 那一格）
    expect(collides(1.5, 1, 1.5, voxels)).toBe(false);
    // 脚陷进地毯里（y=0.01）才算撞
    expect(collides(1.5, 0.01, 1.5, voxels)).toBe(true);
  });

  it("楼梯这类非满方块也挡人", () => {
    // solid=false 表示不挡光，但人有碰撞 —— 之前只认 solid，结果能穿过去。
    const query = solidQuery(world(volumeAt(1, 0, 1)));
    expect(query(1, 0, 1)).toBe(true);
  });
});

describe("collides", () => {
  it("站在地面上不碰", () => {
    const voxels = world(solidAt(1, 0, 1));
    expect(collides(1.5, 1, 1.5, voxels)).toBe(false);
  });

  it("陷进方块里会判定碰撞", () => {
    const voxels = world(solidAt(1, 0, 1));
    expect(collides(1.5, 0, 1.5, voxels)).toBe(true);
  });

  it("半格宽的身体能贴着墙走过去", () => {
    const voxels = world(solidAt(2, 1, 1));
    expect(collides(1.5, 1, 1.5, voxels)).toBe(false);
    expect(collides(1.8, 1, 1.5, voxels)).toBe(true);
  });

  it("头顶碰到方块也会判定碰撞", () => {
    const voxels = world(solidAt(1, 2, 1));
    expect(collides(1.5, 1, 1.5, voxels)).toBe(true);
  });

  it("没有碰撞盒的格子可以穿过（花草这类）", () => {
    const voxels: VoxelMap = new Map([
      [voxelKey(1, 0, 1), { solid: false, collision: [] }],
    ]);
    expect(collides(1.5, 0, 1.5, voxels)).toBe(false);
  });
});

describe("moveAxis", () => {
  it("没有障碍时整段走完", () => {
    const position = new THREE.Vector3(0.5, 1, 0.5);
    const blocked = moveAxis(position, 0.5, "x", new Map());
    expect(blocked).toBe(false);
    expect(position.x).toBeCloseTo(1, 6);
  });

  it("撞墙时停在墙前，不会穿过去", () => {
    const voxels = world(solidAt(2, 1, 0));
    const position = new THREE.Vector3(0.5, 1, 0.5);
    const blocked = moveAxis(position, 2, "x", voxels);
    expect(blocked).toBe(true);
    expect(position.x).toBeLessThan(2 - PLAYER.halfWidth + 0.01);
    expect(position.x).toBeGreaterThan(0.5);
  });

  it("位移为 0 时什么都不做", () => {
    const position = new THREE.Vector3(0.5, 1, 0.5);
    expect(moveAxis(position, 0, "y", world(solidAt(0, 0, 0)))).toBe(false);
    expect(position.y).toBe(1);
  });
});

describe("spawnPoint", () => {
  it("站在实心方块上面，不会陷进去", () => {
    const voxels = world(
      solidAt(1, 0, 1), solidAt(2, 0, 1), solidAt(3, 0, 1),
      solidAt(1, 0, 2), solidAt(2, 0, 2), solidAt(3, 0, 2),
    );
    const spawn = spawnPoint(voxels, bounds, solidQuery(voxels));
    expect(spawn.position.y).toBeCloseTo(1, 6);
    // 包围盒中心是 2.5。
    expect(spawn.position.x).toBeCloseTo(2.5, 6);
    expect(spawn.position.z).toBeCloseTo(2.5, 6);
  });

  it("全都实心时退回包围盒上方", () => {
    const voxels: VoxelMap = new Map();
    for (let x = 0; x <= 4; x += 1) {
      for (let z = 0; z <= 4; z += 1) {
        for (let y = 0; y <= 4; y += 1) {
          voxels.set(voxelKey(x, y, z), {
            solid: true,
            collision: [{ min: [0, 0, 0], max: [1, 1, 1] }],
          });
        }
      }
    }
    const spawn = spawnPoint(voxels, bounds, solidQuery(voxels));
    expect(spawn.position.y).toBeGreaterThan(bounds.maxY);
  });
});
