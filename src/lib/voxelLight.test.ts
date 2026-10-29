import { describe, expect, it } from "vitest";
import {
  ambientOcclusion,
  aoShades,
  boundsOfVoxels,
  brightnessOfAo,
  FACES,
  faceIndex,
  faceOfNormal,
  shouldFlipDiagonal,
  voxelKey,
  type VoxelMap,
} from "./voxelLight";

/** 场景里「这一格实心吗」。 */
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

const nothing = (): boolean => false;

describe("brightnessOfAo", () => {
  it("就是 shulkr 的四个原值，不做色彩空间转换", () => {
    expect(brightnessOfAo(0)).toBe(1);
    expect(brightnessOfAo(1)).toBe(0.7);
    expect(brightnessOfAo(2)).toBe(0.5);
    expect(brightnessOfAo(3)).toBe(0.2);
  });

  it("单调递减，越暗的档位数值越小", () => {
    const values = [0, 1, 2, 3].map(brightnessOfAo);
    for (let index = 1; index < values.length; index += 1) {
      expect(values[index]).toBeLessThan(values[index - 1]);
    }
  });

  it("越界会被夹住", () => {
    expect(brightnessOfAo(-1)).toBe(1);
    expect(brightnessOfAo(9)).toBe(0.2);
  });
});

describe("faceOfNormal", () => {
  it("六个方向的法线都能判对", () => {
    expect(faceOfNormal(0, 1, 0)).toBe("up");
    expect(faceOfNormal(0, -1, 0)).toBe("down");
    expect(faceOfNormal(0, 0, 1)).toBe("south");
    expect(faceOfNormal(0, 0, -1)).toBe("north");
    expect(faceOfNormal(1, 0, 0)).toBe("east");
    expect(faceOfNormal(-1, 0, 0)).toBe("west");
  });

  it("斜法线取绝对值最大的分量", () => {
    expect(faceOfNormal(0.3, 0.9, 0.1)).toBe("up");
    expect(faceOfNormal(0.9, 0.2, 0.3)).toBe("east");
  });
});

describe("FACES / faceIndex", () => {
  it("顺序和 FACE_AXES 一致", () => {
    expect(FACES[0]).toBe("down");
    expect(FACES[1]).toBe("up");
    expect(faceIndex("east")).toBe(5);
  });
});

describe("ambientOcclusion", () => {
  it("周围全空时是 0 档（最亮）", () => {
    expect(ambientOcclusion([0, 1, 0], "up", 0, 0, 0, nothing)).toBe(0);
  });

  it("顶面：紧贴在上方一格偏东的石块会挡掉一个角", () => {
    // 顶面 (0,0,0) 的顶点 (1,1,1)：面外侧是 (0,1,0)，
    // 往东一格 (1,1,0)，往南一格 (0,1,1)。
    const voxels = world([1, 1, 0]);
    expect(ambientOcclusion([1, 1, 1], "up", 0, 0, 0, solidAtOf(voxels))).toBe(1);
  });

  it("顶面：两个侧邻都实心时最暗（3 档）", () => {
    const voxels = world([1, 1, 0], [0, 1, 1]);
    expect(ambientOcclusion([1, 1, 1], "up", 0, 0, 0, solidAtOf(voxels))).toBe(3);
  });

  it("顶面：只有对角实心是 1 档", () => {
    const voxels = world([1, 1, 1]);
    expect(ambientOcclusion([1, 1, 1], "up", 0, 0, 0, solidAtOf(voxels))).toBe(1);
  });

  it("顶点朝另一侧时看的是对面的邻居", () => {
    // 顶点 (0,1,0)：往西 (-1,1,0)，往北 (0,1,-1)。
    const voxels = world([-1, 1, 0]);
    expect(ambientOcclusion([0, 1, 0], "up", 0, 0, 0, solidAtOf(voxels))).toBe(1);
    // 东侧那块不该影响这个顶点。
    const other = world([1, 1, 0]);
    expect(ambientOcclusion([0, 1, 0], "up", 0, 0, 0, solidAtOf(other))).toBe(0);
  });

  it("侧面：朝东的面上方有块是 1 档", () => {
    const voxels = world([1, 1, 0]);
    expect(ambientOcclusion([1, 1, 1], "east", 0, 0, 0, solidAtOf(voxels))).toBe(1);
  });

  it("完全悬空的方块四面都是 0 档，不会平白变暗", () => {
    for (const face of FACES) {
      expect(ambientOcclusion([0.5, 0.5, 0.5], face, 10, 10, 10, nothing)).toBe(0);
    }
  });
});

describe("aoShades", () => {
  it("四个顶点都会算，全空时都是 1", () => {
    const corners: Array<[number, number, number]> = [
      [0, 1, 0],
      [0, 1, 1],
      [1, 1, 1],
      [1, 1, 0],
    ];
    expect(aoShades("up", corners, 0, 0, 0, nothing)).toEqual([1, 1, 1, 1]);
  });

  it("被挡住的角落变暗，其余顶点仍亮", () => {
    const voxels = world([1, 1, 0], [0, 1, 1]);
    const corners: Array<[number, number, number]> = [
      [0, 1, 0],
      [0, 1, 1],
      [1, 1, 1],
      [1, 1, 0],
    ];
    const shades = aoShades("up", corners, 0, 0, 0, solidAtOf(voxels));
    const index = corners.findIndex((c) => c[0] === 1 && c[2] === 1);
    // 3 档就是 shulkr 的 0.2。
    expect(shades[index]).toBe(0.2);
    expect(Math.max(...shades)).toBeGreaterThan(shades[index]);
  });
});

describe("shouldFlipDiagonal", () => {
  it("对角和相等时不翻", () => {
    expect(shouldFlipDiagonal([1, 0.7, 1, 0.7])).toBe(false);
    expect(shouldFlipDiagonal([1, 1, 1, 1])).toBe(false);
  });

  it("ao0+ao2 更小时翻转，避免对角线出现折痕", () => {
    expect(shouldFlipDiagonal([0.5, 0.2, 0.2, 1])).toBe(true);
  });
});

describe("boundsOfVoxels", () => {
  it("三个方向各扩一格", () => {
    expect(boundsOfVoxels(world([0, 0, 0]))).toEqual({
      minX: -1,
      minY: -1,
      minZ: -1,
      maxX: 1,
      maxY: 1,
      maxZ: 1,
    });
  });

  it("空场景给出一个有限的小盒子", () => {
    const bounds = boundsOfVoxels(new Map());
    expect(Number.isFinite(bounds.minX)).toBe(true);
  });

  it("多方块时包住全部", () => {
    const bounds = boundsOfVoxels(world([0, 0, 0], [5, 3, -2]));
    expect(bounds.minX).toBe(-1);
    expect(bounds.maxX).toBe(6);
    expect(bounds.minZ).toBe(-3);
    expect(bounds.maxZ).toBe(1);
  });
});
