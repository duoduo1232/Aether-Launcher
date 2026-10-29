import { describe, expect, it } from "vitest";
import {
  barGeometry,
  memoryTicks,
  snapStepMiB,
  valueFromDrag,
  valueFromPosition,
} from "./memoryBar";

// 16 GiB 的机器，已用 12 GiB，还能分 4 GiB
const machine = { totalMib: 16_384, availableMib: 4096 };

describe("barGeometry", () => {
  it("puts the thumb where the allocation sits and the divider where the system's use ends", () => {
    const geometry = barGeometry(1024, machine);

    expect(geometry.dividerPct).toBeCloseTo(75);
    expect(geometry.thumbPct).toBeCloseTo(6.25);
    expect(geometry.over).toBe(false);
    expect(geometry.overMiB).toBe(0);
  });

  it("sits exactly on the divider when the free memory is allocated in full", () => {
    const geometry = barGeometry(4096, machine);

    expect(geometry.thumbPct).toBeCloseTo(25);
    expect(geometry.overMiB).toBe(0);
    expect(geometry.over).toBe(false);
  });

  it("keeps the thumb moving right as the allocation passes the free memory", () => {
    // 这是之前的 bug：越界越多，另一个手柄越往左，和「多分配该往右」正好相反。
    const geometry = barGeometry(8192, machine);

    expect(geometry.thumbPct).toBeCloseTo(50);
    expect(geometry.over).toBe(true);
    expect(geometry.overMiB).toBe(4096);
  });

  it("never sends the thumb backwards as the value grows", () => {
    let previous = -1;
    for (const value of [512, 1024, 4096, 6144, 8192, 12_288, 16_384]) {
      const { thumbPct } = barGeometry(value, machine);
      expect(thumbPct).toBeGreaterThan(previous);
      previous = thumbPct;
    }
    expect(previous).toBeCloseTo(100);
  });

  it("only calls it over the available memory once the allocation passes it", () => {
    expect(barGeometry(4096, machine).over).toBe(false);
    expect(barGeometry(4608, machine).over).toBe(true);
    expect(barGeometry(4608, machine).overMiB).toBe(512);
  });

  it("clamps a value outside the range instead of drawing off the track", () => {
    expect(barGeometry(1, machine).valueMiB).toBe(512);
    expect(barGeometry(10_000_000, machine).valueMiB).toBe(16_384);
  });

  it("has no divider and no orange zone when the used memory is unknown", () => {
    for (const system of [null, { totalMib: 8192, availableMib: null }]) {
      const geometry = barGeometry(4096, system);
      expect(geometry.usedMib).toBeNull();
      expect(geometry.dividerPct).toBe(0);
      expect(geometry.over).toBe(false);
      expect(geometry.overMiB).toBe(0);
    }
    expect(barGeometry(4096, { totalMib: 8192, availableMib: null }).thumbPct).toBeCloseTo(50);
  });
});

describe("valueFromPosition", () => {
  it("maps a track position straight onto memory, on the same scale as the thumb", () => {
    expect(valueFromPosition(6.25, machine)).toBe(1024);
    expect(valueFromPosition(50, machine)).toBe(8192);
    expect(valueFromPosition(100, machine)).toBe(16_384);
  });

  it("can be dragged past the free memory instead of stopping at it", () => {
    expect(barGeometry(valueFromPosition(50, machine), machine).over).toBe(true);
    expect(valueFromPosition(100, machine)).toBeGreaterThan(
      16_384 - machine.availableMib,
    );
  });

  it("cannot go below the minimum or past the installed memory", () => {
    expect(valueFromPosition(0, machine)).toBe(512);
    expect(valueFromPosition(-20, machine)).toBe(512);
    expect(valueFromPosition(120, machine)).toBe(16_384);
  });

  it("is a plain slider over the whole track when the used memory is unknown", () => {
    expect(valueFromPosition(50, { totalMib: 8192, availableMib: null })).toBe(4096);
    expect(valueFromPosition(100, null)).toBe(16_384);
  });
});

describe("valueFromDrag", () => {
  // 1% 的轨道 = 163.84 MiB；指针取整粒度 32 MiB，所以手柄看上去是连续的
  it("allocates more for a move to the right and less for a move to the left", () => {
    expect(valueFromDrag(1024, 0, machine)).toBe(1024);
    // 1024 + 819.2 = 1843.2，吸附到最近的 512 的倍数
    expect(valueFromDrag(1024, 5, machine)).toBe(2048);
    expect(valueFromDrag(1024, 5, machine, true)).toBe(1856);
    // 4096 少掉 25%（4096）就到了下限，不会变成 0 或负数
    expect(valueFromDrag(4096, -25, machine)).toBe(512);
  });

  it("snaps to multiples of 512 MiB by default", () => {
    for (const pct of [0.7, 1.3, 2.9, 6.1, 11.4, 17.8]) {
      expect(valueFromDrag(1024, pct, machine) % snapStepMiB).toBe(0);
    }
    // 小于半格的移动不会让值动
    expect(valueFromDrag(1024, 0.5, machine)).toBe(1024);
    // 从不在网格上的值出发，第一次移动就落到网格上
    expect(valueFromDrag(1100, 0.1, machine)).toBe(1024);
  });

  it("still reaches the minimum and the end of the track when they are not on the grid", () => {
    const odd = { totalMib: 8448, availableMib: 2304 }; // 8448 = 16.5 × 512
    expect(valueFromDrag(1024, 500, odd)).toBe(8448);
    expect(valueFromDrag(1024, -500, odd)).toBe(512);
  });

  it("crosses the available memory without changing direction", () => {
    const over = valueFromDrag(4096, 25, machine);

    expect(over).toBe(8192);
    expect(barGeometry(over, machine).over).toBe(true);
    // 继续往右仍然是加，手柄也继续往右
    const further = valueFromDrag(over, 25, machine);
    expect(further).toBe(12_288);
    expect(barGeometry(further, machine).thumbPct).toBeGreaterThan(
      barGeometry(over, machine).thumbPct,
    );
  });

  it("comes back out of the orange zone by moving left, the way it went in", () => {
    const start = 8192; // 比可用内存多出 4096

    expect(valueFromDrag(start, -25, machine)).toBe(4096);
    expect(barGeometry(4096, machine).over).toBe(false);

    const further = valueFromDrag(start, -30, machine);
    expect(further).toBeLessThan(4096);
    expect(barGeometry(further, machine).thumbPct).toBeLessThan(
      barGeometry(4096, machine).thumbPct,
    );
  });

  it("moves in steps fine enough to look continuous when Shift turns snapping off, not in the 256 MiB steps of the keyboard", () => {
    const values = [0.1, 0.2, 0.3, 0.4, 0.5].map((pct) => valueFromDrag(1024, pct, machine, true));

    for (const value of values) {
      expect(value % 32).toBe(0);
    }
    expect(new Set(values).size).toBeGreaterThan(1);
    expect(valueFromDrag(1024, 0.5, machine, true)).toBeLessThan(1024 + 256);
  });

  it("stays between the minimum and the installed memory", () => {
    expect(valueFromDrag(1024, -100, machine)).toBe(512);
    expect(valueFromDrag(1024, 500, machine)).toBe(16_384);
  });
});

describe("memoryTicks", () => {
  it("puts a tick on every 512 MiB the thumb can snap to, at the position the thumb would have", () => {
    const ticks = memoryTicks(machine);

    expect(ticks[0].valueMiB).toBe(512);
    expect(ticks.every((tick) => tick.valueMiB % 512 === 0)).toBe(true);
    for (const tick of ticks) {
      expect(tick.pct).toBeCloseTo(barGeometry(tick.valueMiB, machine).thumbPct, 5);
    }
  });

  it("climbs monotonically from left to right", () => {
    const ticks = memoryTicks(machine);

    for (let index = 1; index < ticks.length; index += 1) {
      expect(ticks[index].pct).toBeGreaterThan(ticks[index - 1].pct);
    }
  });

  it("marks whole GiB as major", () => {
    const ticks = memoryTicks(machine);

    expect(ticks.find((tick) => tick.valueMiB === 1024)?.major).toBe(true);
    expect(ticks.find((tick) => tick.valueMiB === 1536)?.major).toBe(false);
  });

  it("thins out on a big machine instead of drawing hundreds of ticks", () => {
    const ticks = memoryTicks({ totalMib: 131_072, availableMib: 65_536 });

    expect(ticks.length).toBeLessThanOrEqual(33);
    expect(ticks[1].valueMiB - ticks[0].valueMiB).toBeGreaterThan(512);
  });

  it("spans the whole track when the used memory is unknown", () => {
    const ticks = memoryTicks({ totalMib: 8192, availableMib: null });

    expect(ticks[0].pct).toBeCloseTo((512 / 8192) * 100);
    expect(ticks[ticks.length - 1].pct).toBeCloseTo(100);
  });
});
