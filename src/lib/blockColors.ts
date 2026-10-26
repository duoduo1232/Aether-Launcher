/**
 * 方块 ID → 预览颜色的对照表。
 *
 * 预览不需要贴图级还原：一个方块一个代表色就足够辨认结构。表里只放常见
 * 方块；没收录的方块按名字哈希成一个稳定的灰彩色，保证同一方块在多次
 * 渲染里颜色一致。色值按 1.20 前后的经典材质目测取样，不做像素级校准。
 */

const BLOCK_COLORS: Readonly<Record<string, string>> = {
  // 石头类
  "minecraft:stone": "#7d7d7d",
  "minecraft:cobblestone": "#7a7a7a",
  "minecraft:deepslate": "#4c4c51",
  "minecraft:cobbled_deepslate": "#4b4b50",
  "minecraft:andesite": "#888c8a",
  "minecraft:granite": "#956756",
  "minecraft:diorite": "#bfbfc2",
  "minecraft:calcite": "#dfded7",
  "minecraft:tuff": "#6c6f5b",
  "minecraft:bedrock": "#565656",
  "minecraft:obsidian": "#14121d",
  "minecraft:blackstone": "#2b2529",
  "minecraft:stone_bricks": "#7b7b7b",
  "minecraft:bricks": "#97584b",
  "minecraft:sandstone": "#d8cb9d",
  "minecraft:end_stone": "#dbdf9f",
  "minecraft:netherrack": "#72262d",
  // 泥土与地表
  "minecraft:dirt": "#866043",
  "minecraft:coarse_dirt": "#7b563c",
  "minecraft:grass_block": "#7cbd6b",
  "minecraft:podzol": "#5c3f1d",
  "minecraft:mud": "#3c3a3d",
  "minecraft:clay": "#a4a8b8",
  "minecraft:gravel": "#857e79",
  "minecraft:sand": "#dbd3a0",
  "minecraft:red_sand": "#c16e31",
  "minecraft:snow_block": "#f0fafa",
  "minecraft:moss_block": "#5a7240",
  // 木头
  "minecraft:oak_log": "#6b5334",
  "minecraft:spruce_log": "#3b2a17",
  "minecraft:birch_log": "#d7d3c5",
  "minecraft:jungle_log": "#55440f",
  "minecraft:acacia_log": "#676157",
  "minecraft:dark_oak_log": "#312710",
  "minecraft:cherry_log": "#3a2a2d",
  "minecraft:mangrove_log": "#4c3a24",
  "minecraft:oak_planks": "#b8945f",
  "minecraft:spruce_planks": "#7a5a35",
  "minecraft:birch_planks": "#d7c185",
  "minecraft:jungle_planks": "#a97e53",
  "minecraft:acacia_planks": "#ba6337",
  "minecraft:dark_oak_planks": "#4a3218",
  "minecraft:cherry_planks": "#e2b0af",
  "minecraft:bamboo_planks": "#c3b565",
  // 功能方块
  "minecraft:crafting_table": "#79553a",
  "minecraft:furnace": "#767676",
  "minecraft:chest": "#a97938",
  "minecraft:barrel": "#8a6f43",
  "minecraft:bookshelf": "#66512f",
  "minecraft:glass": "#c8e8f0",
  "minecraft:tinted_glass": "#2f2a30",
  "minecraft:glowstone": "#f9d49c",
  "minecraft:sea_lantern": "#adc3c0",
  "minecraft:shroomlight": "#ef8b43",
  "minecraft:beacon": "#7de6d2",
  // 矿石与金属
  "minecraft:coal_ore": "#6a6a6a",
  "minecraft:iron_ore": "#8f8f8f",
  "minecraft:gold_ore": "#9a8656",
  "minecraft:diamond_ore": "#7f8f90",
  "minecraft:iron_block": "#d8d8d8",
  "minecraft:gold_block": "#f9ec4e",
  "minecraft:diamond_block": "#5decf5",
  "minecraft:emerald_block": "#2bc458",
  "minecraft:lapis_block": "#1a4b9c",
  "minecraft:redstone_block": "#a90f16",
  "minecraft:coal_block": "#101010",
  "minecraft:copper_block": "#c16d52",
  "minecraft:netherite_block": "#443b3d",
  // 装饰与杂项
  "minecraft:quartz_block": "#e6e4de",
  "minecraft:purpur_block": "#a97ba4",
  "minecraft:prismarine": "#63ab9b",
  "minecraft:terracotta": "#985e43",
  "minecraft:white_terracotta": "#d1b1a1",
  "minecraft:hay_block": "#a68f25",
  "minecraft:melon": "#70981e",
  "minecraft:pumpkin": "#c07615",
  "minecraft:slime_block": "#7ac07a",
  "minecraft:honey_block": "#f9a83c",
  // 混凝土（16 色）
  "minecraft:white_concrete": "#cfd5d6",
  "minecraft:orange_concrete": "#e06101",
  "minecraft:magenta_concrete": "#bd44b3",
  "minecraft:light_blue_concrete": "#2489c7",
  "minecraft:yellow_concrete": "#f1af15",
  "minecraft:lime_concrete": "#70b919",
  "minecraft:pink_concrete": "#ed8dac",
  "minecraft:gray_concrete": "#3e4447",
  "minecraft:light_gray_concrete": "#9d9da2",
  "minecraft:cyan_concrete": "#158991",
  "minecraft:purple_concrete": "#64209c",
  "minecraft:blue_concrete": "#2d2f8f",
  "minecraft:brown_concrete": "#724728",
  "minecraft:green_concrete": "#495b24",
  "minecraft:red_concrete": "#8e2121",
  "minecraft:black_concrete": "#080a0f",
  // 羊毛（常用几色）
  "minecraft:white_wool": "#e9ecec",
  "minecraft:red_wool": "#a12722",
  "minecraft:blue_wool": "#35399d",
  "minecraft:green_wool": "#546d1b",
  "minecraft:black_wool": "#141519",
  // 植被
  "minecraft:oak_leaves": "#48962f",
  "minecraft:spruce_leaves": "#3d5225",
  "minecraft:birch_leaves": "#6a9c4c",
  "minecraft:cherry_leaves": "#f2b1c5",
  "minecraft:warped_stem": "#2b6755",
  "minecraft:crimson_stem": "#7d3838",
};

/** 空气族：这些方块在预览里直接跳过，不生成体素。 */
export function isAir(blockName: string): boolean {
  return blockName.endsWith(":air") || blockName === "minecraft:light";
}

/**
 * 查一个方块的代表色；未收录的方块按名字哈希出稳定颜色。
 * 返回 `#rrggbb`。
 */
export function colorForBlock(blockName: string): string {
  const known = BLOCK_COLORS[blockName];
  if (known) {
    return known;
  }
  const bare = blockName.startsWith("minecraft:")
    ? blockName.slice("minecraft:".length)
    : blockName;
  let hash = 0;
  for (let index = 0; index < bare.length; index += 1) {
    hash = (hash * 31 + bare.charCodeAt(index)) >>> 0;
  }
  const hue = hash % 360;
  const saturation = 18 + (hash % 24);
  const lightness = 38 + ((hash >> 8) % 22);
  return hslToHex(hue, saturation, lightness);
}

function hslToHex(hue: number, saturation: number, lightness: number): string {
  const scaledS = saturation / 100;
  const scaledL = lightness / 100;
  const chroma = (1 - Math.abs(2 * scaledL - 1)) * scaledS;
  const hp = hue / 60;
  const x = chroma * (1 - Math.abs((hp % 2) - 1));
  const [r1, g1, b1] =
    hp < 1
      ? [chroma, x, 0]
      : hp < 2
        ? [x, chroma, 0]
        : hp < 3
          ? [0, chroma, x]
          : hp < 4
            ? [0, x, chroma]
            : hp < 5
              ? [x, 0, chroma]
              : [chroma, 0, x];
  const m = scaledL - chroma / 2;
  const to255 = (value: number) =>
    Math.round((value + m) * 255)
      .toString(16)
      .padStart(2, "0");
  return `#${to255(r1)}${to255(g1)}${to255(b1)}`;
}
