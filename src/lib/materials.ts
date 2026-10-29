/**
 * 投影的材料清单：按方块统计数量，并按 Minecraft 的堆叠规则换算成
 * 「潜影盒 + 组 + 零散个数」。
 *
 * 用于备料：造一座建筑要挖多少方块、带多少个潜影盒过去。
 */
import type { SchematicDetail } from "../ipc";
import { blockDisplayName } from "./blockNames";
import { isAir } from "./blockColors";
/** 潜影盒容量。每个潜影盒 27 格，每格堆到该物品的堆叠上限。 */
const SHULKER_SLOTS = 27;

/** 一个材料的统计行。 */
export type MaterialRow = {
  /** 方块 id，如 `minecraft:cherry_planks`。 */
  block: string;
  /** 显示名，`minecraft:` 前缀已去掉。 */
  label: string;
  /** 总数。 */
  count: number;
  /** 该物品一组多少个：64、16 或 1。 */
  stackSize: number;
  /** 换算好的「盒 / 组 / 个」。 */
  shulkers: number;
  stacks: number;
  loose: number;
};

/**
 * 这类物品一组只有 16 个。
 *
 * 原版里是这些：告示牌、末影珍珠、雪球、鸡蛋、蜂蜜瓶、盔甲架…… 名字里带
 * 下列词的按 16 算。判断保守：高版本常加新物品，漏判会导致备料不足。
 */
const SIXTEEN_STACK_KEYWORDS = [
  "sign",
  "snowball",
  "egg",
  "ender_pearl",
  "armor_stand",
  "banner",
  "honey_bottle",
  "saddle",
  "book", // 书与笔、成书
];

/**
 * 这类物品一组只有 1 个（不可堆叠）。
 *
 * 建筑投影里常见的只有铁砧、船、盔甲、潜影盒本身，其余留着兜底。
 */
const SINGLE_STACK_KEYWORDS = [
  "anvil",
  "boat",
  "minecart",
  "shield",
  "elytra",
  "totem",
  "potion",
  "music_disc",
  "enchanted_book",
  "helmet",
  "chestplate",
  "leggings",
  "boots",
  "sword",
  "pickaxe",
  "axe",
  "shovel",
  "hoe",
  "horse_armor",
  "firework",
  "shulker_box", // 带内容的潜影盒不可堆叠
];

/**
 * 一个物品一组多少个。
 *
 * 顺序很重要：先判 1（不可堆叠），再判 16，其余按 64。
 */
export function stackSizeFor(block: string): number {
  const name = block.startsWith("minecraft:") ? block.slice("minecraft:".length) : block;
  if (SINGLE_STACK_KEYWORDS.some((keyword) => name.includes(keyword))) {
    return 1;
  }
  if (SIXTEEN_STACK_KEYWORDS.some((keyword) => name.includes(keyword))) {
    return 16;
  }
  return 64;
}

/**
 * 把总数换算成「潜影盒 + 组 + 零散」。
 *
 * 不可堆叠的物品（`stackSize === 1`）没有「组」的概念：一个潜影盒装 27 个，
 * 剩下的直接算零散个数。否则 30 个铁砧会被写成「1 盒 + 3 组」，读起来像
 * 3 组铁砧，其实只是 3 个。
 */
export function toShulkers(
  count: number,
  stackSize: number,
): { shulkers: number; stacks: number; loose: number } {
  const perShulker = stackSize * SHULKER_SLOTS;
  const shulkers = Math.floor(count / perShulker);
  const afterShulkers = count - shulkers * perShulker;
  if (stackSize <= 1) {
    return { shulkers, stacks: 0, loose: afterShulkers };
  }
  const stacks = Math.floor(afterShulkers / stackSize);
  const loose = afterShulkers - stacks * stackSize;
  return { shulkers, stacks, loose };
}

/**
 * 统计一个投影的材料清单。
 *
 * 空气不算；同一方块的不同状态（朝向、半砖上下）合并成一行。
 * 按数量从多到少排序，方便一眼看出主料。
 */
export function summariseMaterials(detail: SchematicDetail): MaterialRow[] {
  const totals = new Map<string, number>();
  for (const region of detail.regions) {
    for (const index of region.indices) {
      const entry = region.palette[index];
      if (!entry) {
        continue;
      }
      const block = entry.split("[")[0].trim();
      if (!block || isAir(block)) {
        continue;
      }
      totals.set(block, (totals.get(block) ?? 0) + 1);
    }
  }

  const rows: MaterialRow[] = [];
  for (const [block, count] of totals) {
    const stackSize = stackSizeFor(block);
    rows.push({
      block,
      label: blockDisplayName(block),
      count,
      stackSize,
      ...toShulkers(count, stackSize),
    });
  }
  rows.sort((left, right) => right.count - left.count || left.label.localeCompare(right.label));
  return rows;
}

/** 「盒 / 组 / 个」的紧凑写法，给表格和界面用。 */
export function describeBoxes(row: Pick<MaterialRow, "shulkers" | "stacks" | "loose">): string {
  const parts: string[] = [];
  if (row.shulkers > 0) {
    parts.push(`${row.shulkers} 盒`);
  }
  if (row.stacks > 0) {
    parts.push(`${row.stacks} 组`);
  }
  if (row.loose > 0) {
    parts.push(`${row.loose} 个`);
  }
  return parts.join(" + ") || "0";
}

/** 导出成 CSV 文本（Excel 可直接打开）。 */
export function materialsToCsv(name: string, rows: MaterialRow[]): string {
  const lines = [`投影,${csvCell(name)}`, "", "材料,总数,一组数量,潜影盒,组,零散,换算"];
  for (const row of rows) {
    lines.push(
      [
        csvCell(row.label),
        String(row.count),
        String(row.stackSize),
        String(row.shulkers),
        String(row.stacks),
        String(row.loose),
        csvCell(describeBoxes(row)),
      ].join(","),
    );
  }
  return lines.join("\r\n");
}

/** 一个 CSV 单元格：逗号、引号和换行要转义。 */
function csvCell(value: string): string {
  if (/[",\r\n]/.test(value)) {
    return `"${value.replace(/"/g, '""')}"`;
  }
  return value;
}
