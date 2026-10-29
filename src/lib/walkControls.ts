import { useEffect, type RefObject } from "react";
import * as THREE from "three";
import { voxelKey, type LightBounds, type VoxelMap } from "./voxelLight";

/**
 * 第一视角：自由飞行 + 贴地行走。
 *
 * 键位照原版：
 *
 * | 操作 | 键 |
 * |------|-----|
 * | 前后左右 | W / A / S / D |
 * | 上升 | 空格 |
 * | 下降 / 潜行 | Shift |
 * | 冲刺 | 双击 W（飞行时 Ctrl） |
 * | 跳跃 | 空格（走路时） |
 * | 切换飞行 | 双击空格 |
 * | 转视角 | 按住鼠标拖动 |
 *
 * 刻意**不做**「指针锁定」：投影预览是随手打开的小窗口，锁住鼠标之后想切走
 * 都得先按 Esc。改成按住拖动转视角，手感与 OrbitControls 一致。
 *
 * 碰撞照原版：身体是 0.6 × 1.8 的方柱，眼睛在脚上方 1.62，潜行时降到 1.27。
 * 走路时逐轴推进、撞到就停；飞行时不参与碰撞。
 */

const PLAYER_HALF_WIDTH = 0.3;
const PLAYER_HEIGHT = 1.8;
const EYE_HEIGHT = 1.62;
const SNEAK_EYE_HEIGHT = 1.27;

/** 格/秒。数值取自原版移动速度。 */
const WALK_SPEED = 4.317;
const SPRINT_SPEED = 5.612;
const SNEAK_SPEED = 1.295;
const FLY_SPEED = 10.92;
const FLY_SPRINT_SPEED = 21.84;

const GRAVITY = 32;
const JUMP_SPEED = 9;

/** 视角灵敏度：每像素弧度，约 0.25°/px。 */
const LOOK_SENSITIVITY = 0.0045;

/** 双击判定窗口（毫秒），和原版的「冲刺」一致。 */
const DOUBLE_TAP_MS = 300;

export type CameraMode = "orbit" | "walk";

export type SolidQuery = (x: number, y: number, z: number) => boolean;

/**
 * 从体素表得到一个「这一格挡不挡人」的判定。
 *
 * 用 `collision` 而不是 `solid`：楼梯、台阶、栅栏、墙这些不是满立方体但人有
 * 碰撞；地毯、压力板虽然也挡人，但只有 1/16 高，**能站上去**——所以不能当成
 * 整格。这个函数只回答「这一格有没有碰撞盒」，具体碰撞由 [`collides`] 按
 * 盒子的真实高度算。
 */
export function solidQuery(voxels: VoxelMap): SolidQuery {
  return (x, y, z) =>
    (voxels.get(voxelKey(x, y, z))?.collision.length ?? 0) > 0;
}

/** 玩家的 AABB 是否与任何碰撞盒重叠。 */
export function collides(
  x: number,
  y: number,
  z: number,
  voxels: VoxelMap,
): boolean {
  const pMinX = x - PLAYER_HALF_WIDTH;
  const pMaxX = x + PLAYER_HALF_WIDTH;
  const pMinY = y;
  const pMaxY = y + PLAYER_HEIGHT;
  const pMinZ = z - PLAYER_HALF_WIDTH;
  const pMaxZ = z + PLAYER_HALF_WIDTH;
  const minX = Math.floor(pMinX - 0.5);
  const maxX = Math.floor(pMaxX + 0.5);
  const minY = Math.floor(pMinY - 0.5);
  const maxY = Math.floor(pMaxY + 0.5);
  const minZ = Math.floor(pMinZ - 0.5);
  const maxZ = Math.floor(pMaxZ + 0.5);
  for (let bx = minX; bx <= maxX; bx += 1) {
    for (let by = minY; by <= maxY; by += 1) {
      for (let bz = minZ; bz <= maxZ; bz += 1) {
        // 直接读碰撞盒：`solidQuery` 只回答「这格有没有碰撞盒」，真正
        // 挡不挡由盒子高度决定（地毯 1/16、栅栏 1.5 格高都在这里生效）。
        const boxes = voxels.get(voxelKey(bx, by, bz))?.collision;
        if (boxes === undefined) {
          continue;
        }
        for (const box of boxes) {
          // 盒子的局部坐标 → 世界坐标。
          const boxMinX = bx + box.min[0];
          const boxMaxX = bx + box.max[0];
          const boxMinY = by + box.min[1];
          const boxMaxY = by + box.max[1];
          const boxMinZ = bz + box.min[2];
          const boxMaxZ = bz + box.max[2];
          if (
            pMaxX > boxMinX &&
            pMinX < boxMaxX &&
            pMaxY > boxMinY &&
            pMinY < boxMaxY &&
            pMaxZ > boxMinZ &&
            pMinZ < boxMaxZ
          ) {
            return true;
          }
        }
      }
    }
  }
  return false;
}

/**
 * 沿一个轴推进一段距离，撞墙就停在墙前。
 *
 * 逐轴分开处理是 Minecraft 的做法。这里在每一步之后再查一次是否重叠，
 * 而不是去算精确的接触面 —— 精度够用（每帧位移远小于一格），代码少很多。
 * 返回是否被挡住，调用方据此把该轴的速度清零。
 */
export function moveAxis(
  position: THREE.Vector3,
  delta: number,
  axis: "x" | "y" | "z",
  voxels: VoxelMap,
): boolean {
  if (delta === 0) {
    return false;
  }
  const before = position[axis];
  position[axis] = before + delta;
  if (!collides(position.x, position.y, position.z, voxels)) {
    return false;
  }
  // 退回到「刚好不重叠」的位置：二分几次就足够，省得精确求接触面。
  let low = 0;
  let high = delta;
  for (let step = 0; step < 12; step += 1) {
    const mid = (low + high) / 2;
    position[axis] = before + mid;
    if (collides(position.x, position.y, position.z, voxels)) {
      high = mid;
    } else {
      low = mid;
    }
  }
  position[axis] = before + low;
  return true;
}

/**
 * 选出第一视角的出生点：从包围盒中心正上方往下找第一个能站人的空位。
 *
 * 直接放在包围盒中心多半陷在墙里或者悬在半空。这里竖直往下扫描，找到「这一格
 * 空、脚下实心」的位置就站上去；一格都找不到就停在顶面上方向外一点。
 */
export function spawnPoint(
  voxels: VoxelMap,
  bounds: LightBounds,
  solid: SolidQuery = solidQuery(voxels),
): { position: THREE.Vector3; yaw: number } {
  const x = (bounds.minX + bounds.maxX + 1) / 2;
  const z = (bounds.minZ + bounds.maxZ + 1) / 2;
  for (let y = bounds.maxY + 2; y >= bounds.minY - 1; y -= 1) {
    if (
      !collides(x, y, z, voxels) &&
      solid(Math.floor(x), y - 1, Math.floor(z))
    ) {
      // 站在 y 这一格的底面上，视线朝南（-Z，原版方向）。
      return { position: new THREE.Vector3(x, y, z), yaw: 0 };
    }
  }
  return {
    position: new THREE.Vector3(x, bounds.maxY + 2, z),
    yaw: 0,
  };
}

type Options = {
  camera: THREE.PerspectiveCamera;
  dom: HTMLElement;
  voxels: VoxelMap;
  bounds: LightBounds;
  /** 是否启用第一视角。关掉时停止监听键盘鼠标。 */
  active: boolean;
  /** 滑块灵敏度倍率（1 = 默认 0.25°/px）。 */
  sensitivity?: number;
  /** 状态变化（切飞行、潜行）时通知界面。 */
  onState?: (state: { flying: boolean }) => void;
};

/**
 * 创建第一视角控制器，返回一个「每帧调用一次」的更新函数与清理函数。
 *
 * 不做 React 状态：位置和速度每帧都变，走 state 会每帧重渲染。全部放在闭包里，
 * 只把「是否飞行」这种低频状态汇报出去。
 */
export function createWalkController(options: Options): {
  update: (delta: number) => void;
  dispose: () => void;
  state: () => {
    flying: boolean;
    sneaking: boolean;
    yaw: number;
    pitch: number;
  };
} {
  const { camera, dom, voxels, bounds, active, onState } = options;
  const sensitivity = options.sensitivity ?? 1;
  const solid = solidQuery(voxels);
  const spawn = spawnPoint(voxels, bounds, solid);

  const position = spawn.position.clone();
  let yaw = spawn.yaw;
  let pitch = -0.15;
  const velocity = new THREE.Vector3();
  let flying = false;
  let sneaking = false;
  let onGround = false;
  let lastWTap = 0;
  let lastSpaceTap = 0;
  let sprinting = false;
  const pressed = new Set<string>();

  if (!active) {
    return {
      update: () => undefined,
      dispose: () => undefined,
      state: () => ({ flying, sneaking, yaw, pitch }),
    };
  }

  const forward = new THREE.Vector3();
  const right = new THREE.Vector3();

  const applyCamera = () => {
    // 眼高：潜行时压低半个头，和原版一致。
    const eye = sneaking ? SNEAK_EYE_HEIGHT : EYE_HEIGHT;
    camera.position.set(position.x, position.y + eye, position.z);
    // 先绕 Y 转 yaw，再绕自身 X 转 pitch。
    camera.rotation.order = "YXZ";
    camera.rotation.set(pitch, yaw, 0);
  };
  applyCamera();

  const onKeyDown = (event: KeyboardEvent) => {
    // 长按会以几十毫秒的间隔**反复**触发 keydown。不滤掉的话，双击判定会把
    // 连续的自动重复当成「又点了一次」—— 长按空格就是在高速反复切飞行（人
    // 上下抽搐），松开时如果恰好落在非飞行状态就直接掉下去。
    if (event.repeat) {
      return;
    }
    const key = event.key.toLowerCase();
    // 输入框里打字不该驱动相机。
    const target = event.target as HTMLElement | null;
    if (
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      target?.isContentEditable === true
    ) {
      return;
    }
    if (
      ["w", "a", "s", "d", " ", "shift", "control", "escape"].includes(key)
    ) {
      event.preventDefault();
    }
    if (key === "w") {
      const now = performance.now();
      // `pressed` 已在过滤 `repeat` 之后，这里只认真实按下。
      if (now - lastWTap < DOUBLE_TAP_MS && !sprinting) {
        sprinting = true;
      }
      lastWTap = now;
    }
    if (key === " ") {
      const now = performance.now();
      if (now - lastSpaceTap < DOUBLE_TAP_MS) {
        // 双击空格：切飞行。清掉速度，并**把这次按下吃掉** —— 否则下面
        // `pressed.add(" ")` 会让人一进飞行就往上冲，看上去像没切成功。
        flying = !flying;
        velocity.set(0, 0, 0);
        onState?.({ flying });
        lastSpaceTap = 0;
        pressed.delete(" ");
        return;
      }
      lastSpaceTap = now;
    }
    pressed.add(key);
  };

  const onKeyUp = (event: KeyboardEvent) => {
    const key = event.key.toLowerCase();
    pressed.delete(key);
    if (key === "w") {
      sprinting = false;
      // 松开就把上一次按下时间清掉。不清的话，"走路 → 松手 → 不到 300ms
      // 再按 W" 会被判成双击，人就莫名其妙开始冲刺 —— 这就是「长按 W 一直
      // 走，中间停一下再走就加速」的来源。
      lastWTap = 0;
    }
  };

  const onBlur = () => {
    pressed.clear();
    sprinting = false;
    // 切走再切回来时别把两次不相关的按键并成「双击」。
    lastWTap = 0;
    lastSpaceTap = 0;
  };

  let dragging = false;
  let pointerId: number | null = null;
  let lastX = 0;
  let lastY = 0;

  const onPointerDown = (event: PointerEvent) => {
    if (event.button !== 0) {
      return;
    }
    dragging = true;
    pointerId = event.pointerId;
    lastX = event.clientX;
    lastY = event.clientY;
    dom.setPointerCapture?.(event.pointerId);
    dom.style.cursor = "grabbing";
  };

  const onPointerMove = (event: PointerEvent) => {
    if (!dragging || event.pointerId !== pointerId) {
      return;
    }
    const dx = event.clientX - lastX;
    const dy = event.clientY - lastY;
    lastX = event.clientX;
    lastY = event.clientY;
    // 原版：鼠标往右，视角往右转（yaw 减小）。
    yaw -= dx * LOOK_SENSITIVITY * sensitivity;
    pitch -= dy * LOOK_SENSITIVITY * sensitivity;
    // 原版俯仰限制是 ±90°，留一点余量免得万向锁。
    const limit = Math.PI / 2 - 0.01;
    pitch = Math.max(-limit, Math.min(limit, pitch));
  };

  const endDrag = () => {
    if (!dragging) {
      return;
    }
    dragging = false;
    if (pointerId !== null) {
      dom.releasePointerCapture?.(pointerId);
      pointerId = null;
    }
    dom.style.cursor = "grab";
  };

  const onPointerUp = () => endDrag();

  dom.addEventListener("keydown", onKeyDown);
  dom.addEventListener("keyup", onKeyUp);
  dom.addEventListener("blur", onBlur);
  dom.addEventListener("pointerdown", onPointerDown);
  dom.addEventListener("pointermove", onPointerMove);
  dom.addEventListener("pointerup", onPointerUp);
  dom.addEventListener("pointercancel", onPointerUp);
  dom.tabIndex = 0;
  dom.style.cursor = "grab";
  dom.focus();

  let removed = false;
  const update = (delta: number) => {
    if (removed) {
      return;
    }
    // 每帧最多推进 50ms，切标签页回来时才不会瞬移穿墙。
    const dt = Math.min(Math.max(delta, 0), 0.05);

    // 水平朝向：朝 -Z 是 yaw = 0。
    forward.set(-Math.sin(yaw), 0, -Math.cos(yaw));
    right.set(Math.cos(yaw), 0, -Math.sin(yaw));

    const wish = new THREE.Vector3();
    if (pressed.has("w")) wish.add(forward);
    if (pressed.has("s")) wish.sub(forward);
    if (pressed.has("d")) wish.add(right);
    if (pressed.has("a")) wish.sub(right);
    if (wish.lengthSq() > 0) {
      wish.normalize();
    }

    sneaking = pressed.has("shift") && !flying;
    const boost = pressed.has("control");

    if (flying) {
      const speed = sprinting || boost ? FLY_SPRINT_SPEED : FLY_SPEED;
      // 飞行时 Shift 是下降，Ctrl 是加速，和原版一致。
      const vertical =
        (pressed.has(" ") ? 1 : 0) - (pressed.has("shift") ? 1 : 0);
      velocity.set(wish.x * speed, vertical * speed, wish.z * speed);
      position.x += velocity.x * dt;
      position.y += velocity.y * dt;
      position.z += velocity.z * dt;
    } else {
      const speed = sneaking
        ? SNEAK_SPEED
        : sprinting
          ? SPRINT_SPEED
          : WALK_SPEED;
      velocity.x = wish.x * speed;
      velocity.z = wish.z * speed;

      if (pressed.has(" ") && onGround) {
        velocity.y = JUMP_SPEED;
        onGround = false;
      }
      velocity.y -= GRAVITY * dt;

      // 逐轴推进：先 Y 再水平，落地判定的结果才可靠。
      const blockedY = moveAxis(position, velocity.y * dt, "y", voxels);
      if (blockedY) {
        if (velocity.y < 0) {
          onGround = true;
        }
        velocity.y = 0;
      } else {
        onGround = false;
      }
      moveAxis(position, velocity.x * dt, "x", voxels);
      moveAxis(position, velocity.z * dt, "z", voxels);
    }

    // 掉出场景就送回去，别让人对着虚空发呆。
    if (position.y < bounds.minY - 32) {
      position.copy(spawn.position);
      velocity.set(0, 0, 0);
    }

    applyCamera();
  };


  const dispose = () => {
    removed = true;
    dom.removeEventListener("keydown", onKeyDown);
    dom.removeEventListener("keyup", onKeyUp);
    dom.removeEventListener("blur", onBlur);
    dom.removeEventListener("pointerdown", onPointerDown);
    dom.removeEventListener("pointermove", onPointerMove);
    dom.removeEventListener("pointerup", onPointerUp);
    dom.removeEventListener("pointercancel", onPointerUp);
    dom.style.cursor = "";
  };

  return {
    update,
    dispose,
    state: () => ({ flying, sneaking, yaw, pitch }),
  };
}

/**
 * 把键盘焦点交给画布。第一视角必须在可聚焦元素上监听 keydown，
 * 否则用户点了别处就再也走不动。
 */
export function useAutoFocus(ref: RefObject<HTMLElement | null>): void {
  useEffect(() => {
    ref.current?.focus();
  }, [ref]);
}

/** 供测试用的常量。 */
export const PLAYER = {
  halfWidth: PLAYER_HALF_WIDTH,
  height: PLAYER_HEIGHT,
  eyeHeight: EYE_HEIGHT,
  sneakEyeHeight: SNEAK_EYE_HEIGHT,
} as const;
