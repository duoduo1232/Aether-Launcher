import type { SystemMemory } from "../ipc";
import { minMemoryMiB, sliderMaxMiB, usedMiB } from "./memory";

/**
 * 内存滑块的几何：**一根轨道、一个手柄、一个方向**。
 *
 * 轨道是一条**分配值轴**，左端 0 MiB，右端是这台机器的物理内存（`totalMib`）。
 * 手柄就在 `分配值 / totalMib` 处，所以往右拖永远是多分配、往左拖永远是少分配，
 * 和任何一个普通滑块一样。
 *
 * 轨道上还有两个标记：
 *
 * - **分界线**（`dividerPct`）在 `已用 / total` 处，左边的红区是别的程序占着的内存，
 *   这里只是想说明「还剩多少地方能放」。
 * - **橙色区**从分界线开始往右。手柄一旦越过分界线，说明「已用 + 分配」超过了物理
 *   内存，那一段就是橙的，意思是会挤爆。
 *
 * 两个标记都在手柄右边或左边按同一把尺子量，没有任何一段是反向的。早先的版本把
 * 轨道画成物理内存、又在分界线上放了第二个手柄，越界越多它越往左——这里把它整个
 * 去掉，越界只体现为手柄走进橙区。
 */

export type BarGeometry = {
  /** 轨道右端代表的内存：可分配的上限 */
  totalMib: number;
  /** 其他程序已用的内存；不知道时为 null，此时没有分界线和橙区 */
  usedMib: number | null;
  /** 分界线位置：已用内存占轨道的百分比 */
  dividerPct: number;
  /** 分配值，已夹到可调范围内 */
  valueMiB: number;
  /** 手柄位置：分配值占轨道的百分比 */
  thumbPct: number;
  /** 超出可用内存的部分；不知道已用内存时为 0 */
  overMiB: number;
  /** 橙色区是否可见：手柄越过了分界线 */
  over: boolean;
};

function clamp(value: number, low: number, high: number): number {
  return Math.min(high, Math.max(low, value));
}

export function barGeometry(valueMiB: number, system: SystemMemory | null): BarGeometry {
  const max = sliderMaxMiB(system);
  const totalMib = system?.totalMib ?? max;
  const used = usedMiB(system);
  const pct = (mib: number) => clamp((mib / totalMib) * 100, 0, 100);

  const value = clamp(valueMiB, minMemoryMiB, max);
  const dividerPct = used === null ? 0 : pct(used);
  // 还能再分多少：不分阶段、不取整，只是用来算越界量
  const freeMib = used === null ? totalMib : Math.max(0, totalMib - used);
  const overMiB = used === null ? 0 : Math.max(0, value - freeMib);

  return {
    totalMib,
    usedMib: used,
    dividerPct,
    valueMiB: value,
    thumbPct: pct(value),
    overMiB,
    over: overMiB > 0,
  };
}

/**
 * 指针拖动默认吸附的粒度：512 MiB 的整数倍，和最小值 512 MiB 在同一条网格上。
 * 按住 Shift 拖动时关闭吸附，改用下面的细粒度。
 */
export const snapStepMiB = 512;

/**
 * 不吸附（按住 Shift）时指针拖动的取整粒度。比键盘和设置里用的 256 MiB 细得多：
 * 轨道上 1 像素大约是十几到几十 MiB，用 256 的话手柄会一格一格地跳，不跟手。
 * 32 MiB 约等于 1–2 个像素，看上去是连续的；落到设置里的值仍然是合法的 MiB 整数。
 */
export const pointerStepMiB = 32;

function roundToStep(value: number, fine: boolean): number {
  const step = fine ? pointerStepMiB : snapStepMiB;
  return Math.round(value / step) * step;
}

/**
 * 轨道上的某个位置对应多少内存。
 *
 * 只有一个手柄、一条轴，所以位置和值一一对应，点哪儿就是哪儿。
 */
export function valueFromPosition(
  pct: number,
  system: SystemMemory | null,
  fine = false,
): number {
  const max = sliderMaxMiB(system);
  const totalMib = system?.totalMib ?? max;
  const raw = (clamp(pct, 0, 100) / 100) * totalMib;
  return clamp(roundToStep(raw, fine), minMemoryMiB, max);
}

/**
 * 拖动手柄：相对的，按下时手柄不跳到指针底下，而是保持按下的偏移。
 *
 * `movedPct` 是指针相对按下点移动了多少个轨道百分比，往右为正。位置和值一一对应，
 * 所以就是「原值 + 这一段对应的内存」，没有任何分阶段的分支。
 */
export function valueFromDrag(
  startValueMiB: number,
  movedPct: number,
  system: SystemMemory | null,
  fine = false,
): number {
  const max = sliderMaxMiB(system);
  const totalMib = system?.totalMib ?? max;
  return clamp(
    roundToStep(startValueMiB + (movedPct / 100) * totalMib, fine),
    minMemoryMiB,
    max,
  );
}

/** 轨道上的一根刻度：位置是轨道上的百分比，`valueMiB` 是它代表的内存。 */
export type MemoryTick = { valueMiB: number; pct: number; major: boolean };

/** 刻度最多这么多根，再多在窄轨道上就糊成一片。 */
const maxTicks = 32;

/**
 * 吸附网格对应的刻度：每 512 MiB 一根，位置和手柄用的是同一把尺子，所以刻度就是
 * 手柄会停下的那些地方。内存很大时刻度间隔翻倍（1 GiB、2 GiB…），保证不超过 32 根；
 * 每个整 GiB（间隔翻倍后是每两格）画得更长一些。
 */
export function memoryTicks(system: SystemMemory | null): MemoryTick[] {
  const max = sliderMaxMiB(system);
  const totalMib = system?.totalMib ?? max;
  let step = snapStepMiB;
  while ((max - minMemoryMiB) / step > maxTicks) {
    step *= 2;
  }
  const majorEvery = step === snapStepMiB ? 1024 : step * 2;

  const ticks: MemoryTick[] = [];
  for (let value = minMemoryMiB; value <= max; value += step) {
    ticks.push({
      valueMiB: value,
      pct: clamp((value / totalMib) * 100, 0, 100),
      major: value % majorEvery === 0,
    });
  }
  return ticks;
}
