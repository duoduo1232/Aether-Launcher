/**
 * 方块面的明暗：环境光遮蔽（AO）。
 *
 * 判据照抄 Structura 的 `meshWorker`（shulkr 用的渲染器），不做任何推测：
 *
 * - **不做天空光**。它不扫描遮挡、不区分室内室外，方块亮度只由 AO 决定。
 * - **不做按朝向的固定明暗**。`up` 不比 `east` 亮 —— 原版那种 `shade` 表
 *   它压根没有。整张图的立体感全靠 AO。
 * - **AO 四档**：`[1, 0.7, 0.5, 0.2]`。一个顶点看它周围三格（沿面内两个轴
 *   各走一格、再加对角）：两侧都被挡住 → 3 档（0.2，最暗）；否则取三格实心
 *   的数量。
 *
 * 这样写出来的画面和它在网页里看到的一致，也不需要什么「天空光 / 洞穴光照」
 * 的额外假设 —— 那些是我之前凭印象加的，和参考实现不符。
 *
 * 另外照搬了它的三角形拆分方式：当一个面的两个对角 AO 之和不相等时，换一种
 * 切分方向，避免对角线两侧亮度突变（那个著名的「AO 缝」）。
 */

/**
 * AO 四档亮度，索引即「被挡住的邻居数量」。
 *
 * 来自 shulkr 的 `$t = [1, .7, .5, .2]`。
 */
const AO_BRIGHTNESS = [1, 0.7, 0.5, 0.2];

/**
 * 每个面的两个切向轴，下标对应 `FACES` 的顺序。
 *
 * 来自 shulkr 的 `en = [[1,-1,0,2],[1,1,0,2],[2,-1,0,1],[2,1,0,1],[0,-1,2,1],[0,1,2,1]]`：
 * 每项是 `[法线轴, 法线符号, 切向轴A, 切向轴B]`，轴编号 0=x / 1=y / 2=z。
 */
const FACE_AXES: ReadonlyArray<readonly [number, number, number, number]> = [
  [1, -1, 0, 2], // down
  [1, 1, 0, 2], // up
  [2, -1, 0, 1], // north
  [2, 1, 0, 1], // south
  [0, -1, 2, 1], // west
  [0, 1, 2, 1], // east
];

/** 面编号与名字的对应，和 `FACE_AXES` 同序。 */
export const FACES = ["down", "up", "north", "south", "west", "east"] as const;

export type FaceName = (typeof FACES)[number];

/** 一个轴对齐盒子，坐标是方块内局部坐标 0..1。 */
export type VoxelBox = {
  min: [number, number, number];
  max: [number, number, number];
};

/**
 * 一格方块。
 *
 * `solid` 只用于光照（AO 采样），`collision` 只用于第一视角的碰撞。两者必须
 * 分开，而且碰撞不能只用布尔量：
 *
 * - 楼梯、栅栏不是满方块（不挡光），但人有碰撞；
 * - 地毯、压力板、绊线钩只有 1/16 高 —— 人能**踩上去**，不该被当成整格墙。
 *
 * 所以碰撞存真实盒子（`collision`），空数组表示可以穿过。
 */
export type VoxelInfo = {
  solid: boolean;
  /** 碰撞盒，方块内局部坐标。空数组表示不挡人。 */
  collision: VoxelBox[];
};

/** `x|y|z` → 这一格。 */
export type VoxelMap = Map<string, VoxelInfo>;

export function voxelKey(x: number, y: number, z: number): string {
  return `${x}|${y}|${z}`;
}

/** 场景边界，含 `|x|y|z|` 三个维度。 */
export type LightBounds = {
  minX: number;
  minY: number;
  minZ: number;
  maxX: number;
  maxY: number;
  maxZ: number;
};

/**
 * 从体素索引里量出边界，**三个方向各扩一圈**。
 *
 * 必须外扩：AO 要读面外侧那一格，对建筑最外层的方块来说那格不在包围盒里。
 * 不扩的话所有外表面都读不到邻居，全部算成「无遮挡」，画面会是惨白一片。
 */
export function boundsOfVoxels(
  voxels: VoxelMap,
  fallback?: LightBounds,
): LightBounds {
  let minX = Infinity;
  let minY = Infinity;
  let minZ = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let maxZ = -Infinity;
  for (const key of voxels.keys()) {
    const [x, y, z] = key.split("|").map(Number);
    if (x < minX) minX = x;
    if (y < minY) minY = y;
    if (z < minZ) minZ = z;
    if (x > maxX) maxX = x;
    if (y > maxY) maxY = y;
    if (z > maxZ) maxZ = z;
  }
  if (!Number.isFinite(minX)) {
    return fallback ?? { minX: -1, minY: -1, minZ: -1, maxX: 1, maxY: 1, maxZ: 1 };
  }
  return {
    minX: minX - 1,
    minY: minY - 1,
    minZ: minZ - 1,
    maxX: maxX + 1,
    maxY: maxY + 1,
    maxZ: maxZ + 1,
  };
}

/**
 * 从法线判断这是哪个面。
 *
 * 照抄 shulkr 的 `tn()`：取绝对值最大的分量定轴，再看符号定正反。法线全零
 * （极少数缺数据的模型）时落到 `east`，无所谓 —— 那种面本来就看不见。
 */
export function faceOfNormal(nx: number, ny: number, nz: number): FaceName {
  const ax = Math.abs(nx);
  const ay = Math.abs(ny);
  const az = Math.abs(nz);
  if (ay >= ax && ay >= az) {
    return ny > 0 ? "up" : "down";
  }
  if (az >= ax) {
    return nz > 0 ? "south" : "north";
  }
  return nx > 0 ? "east" : "west";
}

/** 面名 → `FACE_AXES` 的下标。 */
export function faceIndex(face: FaceName): number {
  return FACES.indexOf(face);
}

/**
 * 一个顶点的 AO 档位（0..3）。
 *
 * 参数是顶点在方块坐标系里的位置（0..1）和它所在的面。照 shulkr 的 `nn()`：
 *
 * 1. 以方块中心 (0.5, 0.5, 0.5) 为参照，确定这个顶点在面内两个切向轴上的
 *    方向（±1）；
 * 2. 从**面外侧那一格**出发，沿两个切向各走一格，看是否实心；
 * 3. 对角格也看一次；
 * 4. 两侧都实心 → 3 档；否则取三个格子的实心数量。
 *
 * 注意「面外侧那一格」的偏移由 `FACE_AXES` 给出，不是自己这一格。
 */
export function ambientOcclusion(
  vertex: readonly [number, number, number],
  face: FaceName,
  bx: number,
  by: number,
  bz: number,
  solidAt: (x: number, y: number, z: number) => boolean,
): number {
  const [normalAxis, normalSign, axisU, axisV] = FACE_AXES[faceIndex(face)];
  const corner: [number, number, number] = [vertex[0], vertex[1], vertex[2]];
  const center: [number, number, number] = [0.5, 0.5, 0.5];
  // 顶点在切向两个轴上的方向。
  const su = corner[axisU] >= center[axisU] ? 1 : -1;
  const sv = corner[axisV] >= center[axisV] ? 1 : -1;

  // 面外侧那一格。
  const probe: [number, number, number] = [0, 0, 0];
  probe[normalAxis] = normalSign;

  probe[axisU] = su;
  probe[axisV] = 0;
  const sideU = solidAt(bx + probe[0], by + probe[1], bz + probe[2]);

  probe[axisU] = 0;
  probe[axisV] = sv;
  const sideV = solidAt(bx + probe[0], by + probe[1], bz + probe[2]);

  probe[axisU] = su;
  const diagonal = solidAt(bx + probe[0], by + probe[1], bz + probe[2]);

  if (sideU && sideV) {
    return 3;
  }
  return Number(sideU) + Number(sideV) + Number(diagonal);
}

/**
 * 一个 AO 档位对应的亮度。
 *
 * **直接用 shulkr 的原值** `[1, 0.7, 0.5, 0.2]`，不做任何色彩空间转换。
 *
 * 之前这里套了一层 sRGB→线性，理由是「shulkr 的 WebGL 当最终像素乘，three.js
 * 的顶点色是线性」。那个推理是错的：参考实现的顶点色和纹理**同一个空间**，
 * 乘出来直接就是想要的观感；额外转线性等于把 0.7 压成 0.448，暗角会重得
 * 莫名其妙 —— 画面上就是「到处都有阴影」。
 */
export function brightnessOfAo(ao: number): number {
  const index = Math.max(0, Math.min(3, Math.round(ao)));
  return AO_BRIGHTNESS[index];
}

/**
 * 一个面的四个顶点各自的 AO 亮度。
 *
 * 顶点顺序按 `texturedMesh.pushQuad` 的约定：左上、左下、右下、右上。
 * 每个顶点单独算 AO，就能得到 Minecraft 那种角落发暗的效果。
 */
export function aoShades(
  face: FaceName,
  corners: ReadonlyArray<readonly [number, number, number]>,
  bx: number,
  by: number,
  bz: number,
  solidAt: (x: number, y: number, z: number) => boolean,
): [number, number, number, number] {
  return [
    brightnessOfAo(ambientOcclusion(corners[0], face, bx, by, bz, solidAt)),
    brightnessOfAo(ambientOcclusion(corners[1], face, bx, by, bz, solidAt)),
    brightnessOfAo(ambientOcclusion(corners[2], face, bx, by, bz, solidAt)),
    brightnessOfAo(ambientOcclusion(corners[3], face, bx, by, bz, solidAt)),
  ];
}

/**
 * 三角形拆分是否要翻转对角线。
 *
 * 默认顺序是 `(0,1,2) (0,2,3)`。当一个面的两个对角 AO 之和不相等时，把对角
 * 换成另一条，避免对角线两侧亮度突变 —— 这就是 Minecraft 里那块「折痕」。
 *
 * 判据照抄 shulkr：`ao0 + ao2 < ao1 + ao3` 时翻转。
 */
export function shouldFlipDiagonal(
  shades: readonly [number, number, number, number],
): boolean {
  return shades[0] + shades[2] < shades[1] + shades[3];
}
