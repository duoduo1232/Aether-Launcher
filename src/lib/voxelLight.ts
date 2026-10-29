/**
 * 方块面的明暗，按 Minecraft 原版客户端的规则（白天、满天空光、不算方块光）：
 *
 * 1. **按朝向的固定明暗**（`shade`）：上 1.0、下 0.5、南北 0.8、东西 0.6。
 *    等轴测视角下同时看到顶面和两个侧面，三者亮度不同，建筑才有立体感；
 *    少了它，石英柱、白墙会糊成一片平面。element 写了 `"shade": false`
 *    （十字花草、灯笼提手）就不打这一层。
 * 2. **环境光遮蔽**（AO）：一个顶点看面外侧那一层里的两个侧邻和一个对角，
 *    原版对四格（含面正前方那格）的亮度取平均，被挡住的格算 0.2，于是
 *    0/1/2/3 个遮挡对应 1.0/0.8/0.6/0.4；两个侧邻都挡住时对角视作被挡。
 *    模型的 `ambientocclusion: false`（`block/cross` 一系的植物）不做 AO。
 * 3. 面不在方块边界上（台阶顶面、楼梯踏步、地毯）时，原版在**方块自己那一层**
 *    采样，而不是外侧一格 —— 贴墙的台阶顶面靠墙那条边会变暗。
 *
 * 这些系数都是原版在 sRGB 数值上直接相乘的结果。three.js 在线性空间里做乘法，
 * 所以写进顶点色之前要用 {@link srgbToLinear} 换一次空间，否则 0.6 的暗角
 * 实际只暗到约 0.8，草色、树叶色也会发灰。
 *
 * 三角形拆分：一个面两个对角 AO 之和不相等时换一条对角线，避免亮度沿对角线
 * 出现折痕。
 */

/**
 * AO 四档亮度，索引即「被挡住的格数」。
 *
 * 原版 `AmbientOcclusionFace` 把四格亮度平均，被挡住的格是 0.2、空的是 1.0：
 * `(1+1+1+1)/4`、`(0.2+1+1+1)/4`……所以是 1.0 / 0.8 / 0.6 / 0.4。
 */
const AO_BRIGHTNESS = [1, 0.8, 0.6, 0.4];

/**
 * 原版按朝向的固定明暗（`BlockAndTintGetter.getShade`，主世界）。
 */
const FACE_SHADE: Record<FaceName, number> = {
  down: 0.5,
  up: 1,
  north: 0.8,
  south: 0.8,
  west: 0.6,
  east: 0.6,
};

/** 一个面按朝向应乘的亮度。`shade = false` 的面恒为 1。 */
export function faceShade(face: FaceName, shade = true): number {
  return shade ? FACE_SHADE[face] : 1;
}

/**
 * sRGB 数值 → 线性值。
 *
 * 原版的染色、AO、朝向明暗都是在 sRGB 数值上相乘的；three.js 把贴图解码到
 * 线性空间再乘顶点色，最后编码回 sRGB。顶点色先解码一次，乘出来的观感才和
 * 原版的 `贴图 × 系数` 一致。
 */
export function srgbToLinear(value: number): number {
  if (value <= 0.04045) {
    return value / 12.92;
  }
  return ((value + 0.055) / 1.055) ** 2.4;
}

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
 * 「面外侧那一格」的偏移由 `FACE_AXES` 给出。`onBoundary` 为 `false`（面在
 * 方块内部，如台阶顶面）时改在方块自己那一层采样，和原版一致。
 */
export function ambientOcclusion(
  vertex: readonly [number, number, number],
  face: FaceName,
  bx: number,
  by: number,
  bz: number,
  solidAt: (x: number, y: number, z: number) => boolean,
  onBoundary = true,
): number {
  const [normalAxis, normalSign, axisU, axisV] = FACE_AXES[faceIndex(face)];
  const corner: [number, number, number] = [vertex[0], vertex[1], vertex[2]];
  const center: [number, number, number] = [0.5, 0.5, 0.5];
  // 顶点在切向两个轴上的方向。
  const su = corner[axisU] >= center[axisU] ? 1 : -1;
  const sv = corner[axisV] >= center[axisV] ? 1 : -1;

  // 采样所在的那一层：面在边界上就是外侧一格，否则是自己这一层。
  const probe: [number, number, number] = [0, 0, 0];
  probe[normalAxis] = onBoundary ? normalSign : 0;

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
 * 一个 AO 档位对应的亮度（原版 sRGB 数值）。
 *
 * 写进顶点色之前还要过一次 {@link srgbToLinear}，那一步在网格化的最后统一做，
 * 这里不转。
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
  onBoundary = true,
): [number, number, number, number] {
  const at = (index: number) =>
    brightnessOfAo(
      ambientOcclusion(corners[index], face, bx, by, bz, solidAt, onBoundary),
    );
  return [at(0), at(1), at(2), at(3)];
}

/**
 * 面是否贴在方块边界上（原版 `FACE_CUBIC` 判据的简化）。
 *
 * 四个顶点沿法线轴的坐标都等于 0（朝负向）或 1（朝正向）才算。`corners`
 * 必须是**已经套过 variant 旋转**的方块内坐标。
 */
export function isOnBoundary(
  face: FaceName,
  corners: ReadonlyArray<readonly [number, number, number]>,
): boolean {
  const [normalAxis, normalSign] = FACE_AXES[faceIndex(face)];
  const plane = normalSign > 0 ? 1 : 0;
  return corners.every((corner) => Math.abs(corner[normalAxis] - plane) < 1e-4);
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
