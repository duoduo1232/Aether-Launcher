import type { AssetPack, AssetQuad, AssetVariant, SchematicDetail } from "../ipc";
import { isAir } from "./blockColors";
import {
  aoShades,
  faceOfNormal,
  faceShade,
  isOnBoundary,
  shouldFlipDiagonal,
  srgbToLinear,
  voxelKey,
  type FaceName,
  type VoxelBox,
  type VoxelMap,
} from "./voxelLight";
import { isWaterlogged, pushWaterFilm } from "./waterFilm";

/** 面名 → 邻居格的世界坐标偏移。剔除时用来找「面外侧那一格」。 */
const FACE_OFFSET: Record<string, [number, number, number]> = {
  down: [0, -1, 0],
  up: [0, 1, 0],
  north: [0, 0, -1],
  south: [0, 0, 1],
  west: [-1, 0, 0],
  east: [1, 0, 0],
};


/**
 * 把「投影里的方块」和「从 jar 提取的方块模型」合成可直接喂给 Three 的几何。
 *
 * 和纯色渲染的区别：
 * - 每个方块不再是一个整立方体，而是模型里定义的**若干盒子**（楼梯 15 个面、
 *   栅栏更少），所以半砖、楼梯、栏杆的形状是对的；
 * - 每个面带图集 UV，看到的是真实的木板纹理而不是一块纯色；
 * - 带 `cullface` 的面在邻居占住该方向时整面跳过 —— 一栋房子的内墙面不会
 *   白白绘制；
 * - 按 `opaque` / `cutout` / `translucent` 分三批，透明方块才能正确混合。
 *
 * 全部是纯函数，不依赖 Three，方便单测。
 */

/** 三个渲染层各一份几何数据。 */
export type LayeredGeometry = {
  opaque: LayerBuffers;
  cutout: LayerBuffers;
  translucent: LayerBuffers;
  /** 实际画出的四边形数，用于界面提示。 */
  quadCount: number;
  /**
   * 有多少方块在资源包里找不到模型。
   *
   * 这些方块**不画**（以前会退回成一个纯色立方体，位置和真实模型差半格，
   * 反而让画面看起来错位）。数量只用于界面提示「材质不完整」。
   */
  missingBlocks: number;
  /** 体素表：每格是否挡光、是否挡人。第一视角用它做碰撞。 */
  voxels: VoxelMap;
};

/** 一个渲染层的顶点缓冲。 */
export type LayerBuffers = {
  positions: Float32Array;
  normals: Float32Array;
  uvs: Float32Array;
  /** 顶点色，RGB 三份。草和树叶靠它乘上群系色，其余全是 1。 */
  tints: Float32Array;
  /** 每个顶点三份（三角形索引会展开）。 */
  count: number;
};

const EMPTY_LAYER: LayerBuffers = {
  positions: new Float32Array(0),
  normals: new Float32Array(0),
  uvs: new Float32Array(0),
  tints: new Float32Array(0),
  count: 0,
};

/** 一个方块实例在网格化时需要的信息。 */
type Voxel = {
  x: number;
  y: number;
  z: number;
  /** 方块 id，如 `minecraft:cherry_planks`。 */
  block: string;
  /** 状态字符串里的属性（如 `facing=north,half=top`），用来挑 variant。 */
  state: Record<string, string>;
};

/**
 * 构建几何。
 *
 * `maxQuads` 是安全上限：一座几万方块的建筑可能产生上百万个四边形，
 * 超过就退回纯色渲染，避免把页面卡死。
 */
export function buildTexturedGeometry(
  detail: SchematicDetail,
  pack: AssetPack | null,
  maxQuads = 400_000,
): LayeredGeometry {
  if (!pack) {
    return {
      opaque: EMPTY_LAYER,
      cutout: EMPTY_LAYER,
      translucent: EMPTY_LAYER,
      quadCount: 0,
      missingBlocks: 0,
      voxels: new Map(),
    };
  }

  //
  // 只有**不透明整方块**才能挡住邻居的面。判据来自模型数据本身：模型填满
  // 1×1×1（`fullCube`）且所有面都在不透明层。玻璃、树叶、台阶、楼梯的
  // `fullCube` 是假，草和树叶的面在 cutout 层，两者都不算遮挡。
  //
  // 不要用方块名列表判断：模组方块和没收录的原版方块会被误判成遮挡，
  // 结果是墙上凭空挖洞。
  const voxels = collectVoxels(detail);
  const variantCache = new Map<string, AssetVariant[]>();
  const variantsFor = (voxel: Voxel): AssetVariant[] => {
    const stateKey = `${voxel.block}|${stateSignature(voxel.state)}`;
    let variants = variantCache.get(stateKey);
    if (variants === undefined) {
      variants = pickVariants(pack, voxel.block, voxel.state);
      variantCache.set(stateKey, variants);
    }
    return variants;
  };

  // 剔除只做两档，判据从简：实心满方块挡住任何邻居；同种方块互挡只限整方块。
  //
  // 原版的 `shouldRenderFace` 依赖完整的 `BlockState` 形状数据，我们的模型烘焙
  // 只有 `fullCube` 一个布尔量，仍不足以复刻。剔除是纯性能优化，代价是「判错
  // 一次就挖一个洞」，所以宁可少剔：楼梯、台阶、栅栏、石英柱这些非整方块一律
  // 不按 id 互剔，缺面的观感远差于多画几个面。
  //
  // 两个满立方体相邻时被剔掉的那两面本来就完全重合、肉眼看不出差别，所以这一
  // 档是安全的。剩下的脏活留给 `maxQuads`。
  //
  // 光照：原版的朝向明暗 × 逐顶点 AO，规则见 `voxelLight`。
  const lightVoxels = collectLightVoxels(voxels, variantsFor);
  // 位置 → 方块 id。剔除「同一种方块相邻」时要比 id，光照表里只有布尔量。
  const blockAt = new Map<string, string>();
  for (const item of voxels) {
    blockAt.set(voxelKey(item.x, item.y, item.z), item.block);
  }
  const isSolid = (x: number, y: number, z: number): boolean =>
    lightVoxels.get(voxelKey(x, y, z))?.solid === true;
  const opaque: number[] = [];
  const cutout: number[] = [];
  const translucent: number[] = [];
  let quadCount = 0;
  let missingBlocks = 0;
  for (const voxel of voxels) {
    const variants = variantsFor(voxel);
    if (variants.length === 0) {
      missingBlocks += 1;
      continue;
    }
    for (const variant of variants) {
      for (const quad of variant.model.quads) {
        // 超过上限就停：继续画只会把页面拖死，前面已画的部分仍然可看。
        if (quadCount >= maxQuads) {
          continue;
        }
        // 剔除：面外侧那一格被占住时，这面对外看不见，整面跳过。
        //
        // **方向必须换成世界方向**。`quad.cullface` 是模型里的朝向，而
        // variant 的 `x`/`y` 旋转会把它转到别处：横放的石英柱（`axis=x`，
        // `x:90,y:90`）模型的 west 面转完朝世界 north。拿原始方向去查邻居，
        // 查的是同排下一根柱子 → 同种方块互剔 → 南北两个侧面全没了。
        //
        // 判据照 shulkr 的 `_ = N(neighbour) || (group && sameGroup(neighbour))`：
        // - 实心满方块挡住**任何**邻居；
        // - 同种方块互挡**只对整方块成立**。楼梯、半砖朝向不同或上下叠放时
        //   接触面形状并不一致，按 id 剔除会开洞。玻璃、树叶是整方块，
        //   这条对它们仍然有效。
        if (quad.cullface) {
          const face = worldFace(quad.cullface as FaceName, variant);
          const [dx, dy, dz] = FACE_OFFSET[face] ?? [0, 0, 0];
          const nx = voxel.x + dx;
          const ny = voxel.y + dy;
          const nz = voxel.z + dz;
          const neighbour = lightVoxels.get(voxelKey(nx, ny, nz));
          if (neighbour) {
            const neighbourBlock = blockAt.get(voxelKey(nx, ny, nz));
            const sameFullCube =
              variant.model.fullCube && neighbourBlock === voxel.block;
            if (neighbour.solid || sameFullCube) {
              continue;
            }
          }
        }
        quadCount += 1;
        const target =
          quad.layer === "translucent"
            ? translucent
            : quad.layer === "cutout"
              ? cutout
              : opaque;
        const placed = placeQuad(quad, variant);
        const shades = quadBrightness(placed, quad, variant, voxel, isSolid);
        pushQuad(target, voxel, quad, placed, shades);
      }
    }
    // 含水方块额外叠一层水膜（楼梯、台阶、栅栏的 `waterlogged=true`）。
    if (isWaterlogged(voxel.block, voxel.state)) {
      quadCount += pushWaterFilm(
        translucent,
        {
          x: voxel.x,
          y: voxel.y,
          z: voxel.z,
          isSolid,
          voxels: lightVoxels,
          budget: maxQuads - quadCount,
        },
        pack.atlas,
      );
    }
  }

  return {
    opaque: toLayer(opaque),
    cutout: toLayer(cutout),
    translucent: toLayer(translucent),
    quadCount,
    missingBlocks,
    voxels: lightVoxels,
  };
}
/**
 * 从方块列表建光照用的体素表。
 *
 * 「实心」= 模型填满整格（`fullCube`）且所有面都在 opaque 层。和渲染分层
 * 用的是同一份判据：光照和遮挡不能各判一套，否则会出现「画得出来但一片死黑」
 * 或者「明明实心却透光」。
 *
 * 玻璃、树叶是满立方体但带透明像素，判为半透 —— 光会衰减但不完全阻断。
 */
function collectLightVoxels(
  voxels: Voxel[],
  variantsFor: (voxel: Voxel) => AssetVariant[],
): VoxelMap {
  const map: VoxelMap = new Map();
  for (const voxel of voxels) {
    const variants = variantsFor(voxel);
    if (variants.length === 0) {
      continue;
    }
    const fullCube = variants.every((variant) => variant.model.fullCube);
    const allOpaque = variants.every((variant) =>
      variant.model.quads.every((quad) => quad.layer === "opaque"),
    );
    // 碰撞盒 = 模型所有面顶点合的 AABB。不能只看单个面 —— 整方块的面是
    // 零厚度的，按「单个面三轴都有跨度」判会让石头都不挡人。
    //
    // 也不能只判「有没有体积」然后当整格：地毯、压力板只有 1/16 高，
    // 整格碰撞会让人在它们前面被挡住（原版能直接踩上去）。
    //
    // 花草那种十字面片：所有顶点落在同一竖直平面里，AABB 在某一轴上是
    // 零厚度 → 不产生碰撞盒，和原版一致。
    const collision: VoxelBox[] = [];
    for (const variant of variants) {
      let minX = Infinity;
      let minY = Infinity;
      let minZ = Infinity;
      let maxX = -Infinity;
      let maxY = -Infinity;
      let maxZ = -Infinity;
      for (const quad of variant.model.quads) {
        for (const position of quad.positions) {
          const rotated = rotate(
            position,
            variant.rotationX,
            variant.rotationY,
          );
          minX = Math.min(minX, rotated[0]);
          minY = Math.min(minY, rotated[1]);
          minZ = Math.min(minZ, rotated[2]);
          maxX = Math.max(maxX, rotated[0]);
          maxY = Math.max(maxY, rotated[1]);
          maxZ = Math.max(maxZ, rotated[2]);
        }
      }
      if (!Number.isFinite(minX)) {
        continue;
      }
      // 三个轴都要有跨度才算有体积。细到 1/16 的薄片仍然算（地毯能站人）。
      if (
        maxX - minX <= 1e-6 ||
        maxY - minY <= 1e-6 ||
        maxZ - minZ <= 1e-6
      ) {
        continue;
      }
      collision.push({
        min: [minX, minY, minZ],
        max: [maxX, maxY, maxZ],
      });
    }
    map.set(voxelKey(voxel.x, voxel.y, voxel.z), {
      solid: fullCube && allOpaque,
      collision,
    });
  }
  return map;
}

/**
 * 模型里的面方向，套上 variant 的 `x`/`y` 旋转后变成**世界方向**。
 *
 * 剔除和 AO 都必须要世界方向。模型的 `cullface` 是「模型坐标系里的朝向」，
 * variant 一旋转它就跟着转：横放的石英柱（`axis=x`，`x:90,y:90`）模型的
 * west 面转完朝世界 north。拿模型方向去查邻居，查的永远是错的格子。
 */
function worldFace(face: FaceName, variant: AssetVariant): FaceName {
  const offset = FACE_OFFSET[face];
  if (offset === undefined) {
    return face;
  }
  const [x, y, z] = rotate(offset, variant.rotationX, variant.rotationY, true);
  return faceOfNormal(x, y, z);
}

/** 套过 variant 旋转之后的四边形（方块内坐标 0..1）。 */
type PlacedQuad = {
  positions: [number, number, number][];
  normal: [number, number, number];
  /** 按法线判出的世界朝向（原版 `FaceBakery` 也是这样定 `direction` 的）。 */
  face: FaceName;
};

function placeQuad(quad: AssetQuad, variant: AssetVariant): PlacedQuad {
  const positions = quad.positions.map((position) =>
    rotate(position, variant.rotationX, variant.rotationY),
  );
  const normal = rotate(quad.normal, variant.rotationX, variant.rotationY, true);
  return { positions, normal, face: faceOfNormal(normal[0], normal[1], normal[2]) };
}

/**
 * 四个顶点的亮度：朝向明暗 × AO（原版 sRGB 数值，空间转换在 `toLayer`）。
 *
 * - AO 必须用**旋转后**的顶点和世界朝向。拿模型坐标去配世界朝向，横放的
 *   石英柱、朝东的楼梯，暗角会落在错的那条边上。
 * - 模型 `ambientocclusion: false`（花草）不做 AO；斜着放的面（十字、灯笼
 *   提手）原版也不做 —— 它们的法线不沿坐标轴，没有「外侧那一层」。
 * - element `shade: false` 不乘朝向明暗。
 */
function quadBrightness(
  placed: PlacedQuad,
  quad: AssetQuad,
  variant: AssetVariant,
  voxel: Voxel,
  isSolid: (x: number, y: number, z: number) => boolean,
): [number, number, number, number] {
  const directional = faceShade(placed.face, quad.shade !== false);
  const axisAligned =
    placed.normal.filter((component) => Math.abs(component) > 1e-4).length === 1;
  if (variant.model.ambientOcclusion === false || !axisAligned) {
    return [directional, directional, directional, directional];
  }
  const onBoundary =
    variant.model.fullCube || isOnBoundary(placed.face, placed.positions);
  const ao = aoShades(
    placed.face,
    placed.positions,
    voxel.x,
    voxel.y,
    voxel.z,
    isSolid,
    onBoundary,
  );
  return [
    ao[0] * directional,
    ao[1] * directional,
    ao[2] * directional,
    ao[3] * directional,
  ];
}


/** 收集投影里的所有非空气方块。 */
function collectVoxels(detail: SchematicDetail): Voxel[] {
  const voxels: Voxel[] = [];
  for (const region of detail.regions) {
    const [sx, sy, sz] = region.size;
    if (sx <= 0 || sy <= 0 || sz <= 0) {
      continue;
    }
    const palette = region.palette;
    const indices = region.indices;
    for (let y = 0; y < sy; y += 1) {
      for (let z = 0; z < sz; z += 1) {
        for (let x = 0; x < sx; x += 1) {
          const index = (y * sz + z) * sx + x;
          if (index >= indices.length) {
            continue;
          }
          const entry = palette[indices[index]] ?? "";
          if (!entry || isAir(entry)) {
            continue;
          }
          const { block, state } = parsePaletteEntry(entry);
          voxels.push({
            x: region.position[0] + x,
            y: region.position[1] + y,
            z: region.position[2] + z,
            block,
            state,
          });
        }
      }
    }
  }
  return voxels;
}

/**
 * 调色板条目可能是 `minecraft:oak_stairs[facing=north,half=top]` 这种带属性的
 * 形式（Litematica 会把方块状态写进去），拆成 id + 属性表。
 */
export function parsePaletteEntry(entry: string): {
  block: string;
  state: Record<string, string>;
} {
  const bracket = entry.indexOf("[");
  if (bracket < 0) {
    return { block: entry.trim(), state: {} };
  }
  const block = entry.slice(0, bracket).trim();
  const inner = entry.slice(bracket + 1, entry.lastIndexOf("]"));
  const state: Record<string, string> = {};
  for (const pair of inner.split(",")) {
    const [name, value] = pair.split("=");
    if (name && value) {
      state[name.trim()] = value.trim();
    }
  }
  return { block, state };
}

/**
 * 从方块的 variant 列表里挑出当前属性命中的全部变体。
 *
 * 为什么是「全部」而不是「一个」：blockstate 有两种写法。
 * - `variants`（工作台、石头）每个属性组合对应**一个**模型，正常情况下只有一条命中；
 * - `multipart`（楼梯、栅栏、玻璃板）由多个**可以同时叠加**的部件组成，
 *   比如楼梯的 `bottom+straight+north` 命中底座和踏板两个部件。
 *   只取一个的话，楼梯只剩半截，或者叠成转角楼梯。
 * 筛选规则（照 shulkr 的 `Be()` / `ze()`）：
 * - blockstate 是 `variants` 写法（`exclusive: true`）：每个属性组合对应**一个**
 *   模型。命中多条时只取条件最多（最精确）的一条。`cherry_shelf` 同时有
 *   「无条件 facing」和「facing + powered + side_chain」两套，全取会叠在一起。
 * - blockstate 是 `multipart` 写法（`exclusive: false`）：部件可以叠加，全取。
 * - `when` 里每一条都必须命中；值可以写成 `"north|east"` 表示任一。
 * - `when` 为空表示「总是适用」。
 */
export function pickVariants(
  pack: AssetPack,
  block: string,
  state: Record<string, string>,
): AssetVariant[] {
  const variants = pack.blocks[block];
  if (!variants || variants.length === 0) {
    return [];
  }

  const matches = (variant: AssetVariant): boolean =>
    variant.when.every(([name, wanted]) => {
      const actual = state[name];
      if (actual === undefined) {
        return false;
      }
      return wanted.split("|").includes(actual);
    });

  const matched = variants.filter(matches);
  if (matched.length > 0) {
    // `variants` 写法只取最精确的一条；`multipart` 全部保留（部件叠加）。
    const exclusive = matched.filter((variant) => variant.exclusive);
    if (exclusive.length === 0) {
      return matched;
    }
    return [
      exclusive.reduce((best, variant) =>
        variant.when.length > best.when.length ? variant : best,
      ),
    ];
  }

  // 属性不全（老投影没写 Properties）时才走到这里：取条件最少的那条，
  // 至少让方块画得出来，形状可能不对但不会整块消失。
  const fallback = variants.reduce((fewest, variant) =>
    variant.when.length < fewest.when.length ? variant : fewest,
  );
  return [fallback];
}

function stateSignature(state: Record<string, string>): string {
  return Object.keys(state)
    .sort()
    .map((name) => `${name}=${state[name]}`)
    .join(",");
}

/**
 * 把一个已经旋转好的四边形平移到体素位置，推进缓冲。
 */
function pushQuad(
  target: number[],
  voxel: Voxel,
  quad: AssetQuad,
  placed: PlacedQuad,
  shades: [number, number, number, number],
): void {
  const rotated = placed.positions;
  const normals = placed.normal;

  // 三角形拆分方向要按 AO 决定，不能固定 (0,1,2)(0,2,3)。
  //
  // 一个四边形切两刀有两种切法，对角线不同。如果对角线两端亮度差很多，
  // 亮度就会沿对角线出现一条生硬的折痕 —— 地砖上那种「到处都有阴影」的
  // 细碎纹路就是它。shulkr 的 `sn()` 在这里判断：
  // `ao0 + ao2 < ao1 + ao3` 时换成另一条对角线。
  const flip = shouldFlipDiagonal(shades);
  const ORDER = flip ? [1, 2, 3, 1, 3, 0] : [0, 1, 2, 0, 2, 3];

  for (const index of ORDER) {
    const position = rotated[index];
    target.push(
      voxel.x + position[0],
      voxel.y + position[1],
      voxel.z + position[2],
    );
    target.push(normals[0], normals[1], normals[2]);
    target.push(quad.uvs[index][0], quad.uvs[index][1]);
    // 顶点色 = 方块染色 × 亮度，此时还是原版的 sRGB 数值；`toLayer` 统一
    // 换成线性值交给 three.js。
    const tint = quad.tint ?? [1, 1, 1];
    const shade = shades[index] ?? 1;
    target.push(tint[0] * shade, tint[1] * shade, tint[2] * shade);
  }
}

/**
 * 按 Minecraft 的规则旋转：先绕 X（0/90/180/270），再绕 Y。
 *
 * 旋转是绕方块中心的，坐标在 0..1 空间里，所以先平移到 -0.5..0.5。
 */
function rotate(
  value: [number, number, number] | readonly number[],
  rotationX: number,
  rotationY: number,
  isNormal = false,
): [number, number, number] {
  let [x, y, z] = [value[0], value[1], value[2]];
  if (!isNormal) {
    x -= 0.5;
    y -= 0.5;
    z -= 0.5;
  }

  // 旋转矩阵必须与 Minecraft 一致，否则几何和法线会一起翻向 —— 面朝里、
  // 贴图镜像，看起来就是「材质和方块对不上」。
  const radiansX = (rotationX * Math.PI) / 180;
  if (radiansX !== 0) {
    const cos = Math.round(Math.cos(radiansX));
    const sin = Math.round(Math.sin(radiansX));
    // 绕 X 轴：y' = y·cos + z·sin，z' = -y·sin + z·cos。
    const ny = y * cos + z * sin;
    const nz = -y * sin + z * cos;
    y = ny;
    z = nz;
  }

  const radiansY = (rotationY * Math.PI) / 180;
  if (radiansY !== 0) {
    const cos = Math.round(Math.cos(radiansY));
    const sin = Math.round(Math.sin(radiansY));
    // 绕 Y 轴：x' = x·cos - z·sin，z' = x·sin + z·cos。
    const nx = x * cos - z * sin;
    const nz = x * sin + z * cos;
    x = nx;
    z = nz;
  }

  if (!isNormal) {
    x += 0.5;
    y += 0.5;
    z += 0.5;
  }
  return [x, y, z];
}

/** `[px,py,pz, nx,ny,nz, u,v] × 6` → 三份独立缓冲。 */
function toLayer(values: number[]): LayerBuffers {
  const count = values.length / 11;
  const positions = new Float32Array(count * 3);
  const normals = new Float32Array(count * 3);
  const uvs = new Float32Array(count * 2);
  const tints = new Float32Array(count * 3);
  for (let index = 0; index < count; index += 1) {
    const base = index * 11;
    positions[index * 3] = values[base];
    positions[index * 3 + 1] = values[base + 1];
    positions[index * 3 + 2] = values[base + 2];
    normals[index * 3] = values[base + 3];
    normals[index * 3 + 1] = values[base + 4];
    normals[index * 3 + 2] = values[base + 5];
    uvs[index * 2] = values[base + 6];
    uvs[index * 2 + 1] = values[base + 7];
    // 原版的染色与明暗是 sRGB 数值上的乘法；three.js 在线性空间里乘顶点色，
    // 这里换一次空间，两边的结果才一致（见 `voxelLight.srgbToLinear`）。
    tints[index * 3] = srgbToLinear(values[base + 8]);
    tints[index * 3 + 1] = srgbToLinear(values[base + 9]);
    tints[index * 3 + 2] = srgbToLinear(values[base + 10]);
  }
  return { positions, normals, uvs, tints, count };
}
