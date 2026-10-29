import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { AssetPack, SchematicDetail } from "../ipc";
import {
  buildTexturedGeometry,
  type LayerBuffers,
} from "../lib/texturedMesh";
import { createWalkController } from "../lib/walkControls";
import { boundsOfVoxels } from "../lib/voxelLight";

type ViewMode = "orbit" | "walk";

/** 切换视角的自定义事件名：渲染循环在 effect 里，React 通过它发指令。 */
const VIEW_MODE_EVENT = "aether-view-mode";

type SchematicPreviewProps = {
  detail: SchematicDetail;
  /**
   * 从版本 jar 提取的方块资源包。为空时什么都不画，由界面提示去提取材质。
   */
  pack?: AssetPack | null;
  /** 拿不到贴图的方块数（材质版本与投影版本不一致时通常很多）。 */
  onMissingBlocks?: (count: number) => void;
};

/**
 * 投影图的 3D 预览。
 *
 * 只有一条渲染路径：按真实方块模型展开成带 UV 的三角形，一张图集贴图，
 * 按 `opaque` / `cutout` / `translucent` 分三批绘制。楼梯、半砖、栏杆的形状
 * 和纹理都来自 Minecraft 自己的模型定义。
 *
 * 两种视角：
 *
 * - **环绕**（默认）：正交等轴测。投影图是方格建筑，透视会让远处缩小、看起来
 *   是歪的。拖动旋转、滚轮缩放、右键平移。
 * - **第一视角**：透视相机，WASD 走动、空格上升、Shift 下降、双击 W 冲刺。
 *   键位和碰撞都在 `walkControls` 里。
 *
 * 光照**不用 three.js 的光源**：方块明暗由 `voxelLight` 算好写进顶点色
 * （原版的朝向明暗 × AO × 群系色，已换成线性值）。用 Lambert 光照的话整张图
 * 会随视角明暗变化，和 Minecraft 那种完全固定的方块明暗对不上。
 */
export function SchematicPreview({
  detail,
  pack = null,
  onMissingBlocks,
}: SchematicPreviewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [failed, setFailed] = useState(false);
  const [mode, setMode] = useState<ViewMode>("orbit");
  const reportMissing = useRef(onMissingBlocks);
  useEffect(() => {
    reportMissing.current = onMissingBlocks;
  }, [onMissingBlocks]);

  // 视角模式是 React 状态（要给按钮显示），渲染循环读的是事件。两边靠这个
  // effect 同步：状态一变就发事件，循环里的 applyMode 收到后换相机。
  useEffect(() => {
    containerRef.current?.dispatchEvent(
      new CustomEvent<ViewMode>(VIEW_MODE_EVENT, { detail: mode }),
    );
  }, [mode]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    // 只有一条渲染路径：真实模型 + 贴图。
    //
    // 以前还有一条「纯色方块」回退（一个方块一个彩色立方体）。它会让预览在
    // 缺少材质时显示成一片色块，而且和真实模型的坐标口径差半格，看起来就是
    // 位置错。现在没有资源包就什么都不画，由界面提示去提取材质。
    if (!pack) {
      return;
    }
    const textured = buildTexturedGeometry(detail, pack);
    // 材质版本和投影版本不一致时，这里会是一个很大的数：那些方块找不到
    // 贴图就不画了，画面上就是「墙漏风」。把数量报给界面，别让人猜。
    reportMissing.current?.(textured.missingBlocks);
    if (textured.quadCount === 0) {
      return;
    }

    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({ antialias: true });
    } catch {
      queueMicrotask(() => setFailed(true));
      return;
    }
    renderer.setPixelRatio(window.devicePixelRatio);
    renderer.setSize(container.clientWidth, container.clientHeight);
    renderer.setClearColor(0x1b1d23, 1);
    // 第一视角要在 canvas 上监听 keydown，而 canvas 默认不可聚焦 ——
    // 不设 tabindex 的话，点了画布也拿不到键盘事件，人就走不动。
    renderer.domElement.tabIndex = 0;
    renderer.domElement.style.outline = "none";
    container.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x1b1d23);

    // 场景按**真实包围盒**取景，不能一直盯原点：投影图里的方块用世界绝对
    // 坐标（y 可能从 60 起、z 从 183 起），盯着 (0,0,0) 会把建筑推出视野。
    const bounds = boundsOfLayers(textured);
    const center = bounds.center;
    const extent = bounds.radius;

    const startAspect =
      container.clientWidth / Math.max(1, container.clientHeight);
    const half = extent * 1.15;
    const camera = new THREE.OrthographicCamera(
      -half * startAspect,
      half * startAspect,
      half,
      -half,
      0.1,
      Math.max(200, extent * 40),
    );
    // 等轴测：从 (1, 0.8, 1) 方向看，对准包围盒中心。
    camera.position.set(
      center.x + extent * 1.5,
      center.y + extent * 1.2,
      center.z + extent * 1.5,
    );
    camera.lookAt(center);

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.08;
    controls.target.copy(center);
    // 正交相机的 `zoom` 只改视口、不改相机到目标的距离，而 OrbitControls 的
    // 灵敏度只看距离 —— 于是放大后拖起来和远景一样快，稍一动画面就飞出去。
    //
    // 但也不能按 zoom 除到底：那样放大 10 倍后平移只剩 1/10，右键根本拖不动。
    // 按**平方根**收，放大 10 倍仍保留约 32% 速度，手感是「慢慢变慢」。
    const applySensitivity = () => {
      const damping = 1 / Math.sqrt(Math.max(1, camera.zoom));
      controls.rotateSpeed = 0.9 * damping;
      controls.panSpeed = 1.2 * damping;
    };
    applySensitivity();
    controls.addEventListener("change", applySensitivity);
    controls.update();

    const disposables: Disposable[] = [];

    addTexturedMeshes(scene, textured, pack, disposables);
    addGrid(scene, bounds, disposables);

    // 第一视角用一台独立的透视相机：人眼就是透视的，正交会在走动时把远近
    // 压成一样大。两台相机同时存在，切换只是换 `render` 的入参。
    const walkCamera = new THREE.PerspectiveCamera(
      70,
      startAspect,
      0.05,
      Math.max(400, extent * 60),
    );
    const walkVoxels = textured.voxels;
    const walkBounds = boundsOfVoxels(walkVoxels, {
      minX: Math.floor(center.x - extent),
      minY: Math.floor(center.y - extent),
      minZ: Math.floor(center.z - extent),
      maxX: Math.ceil(center.x + extent),
      maxY: Math.ceil(center.y + extent),
      maxZ: Math.ceil(center.z + extent),
    });
    let walker: ReturnType<typeof createWalkController> | null = null;
    let viewMode: ViewMode = "orbit";
    let activeCamera: THREE.Camera = camera;

    const applyMode = (next: ViewMode) => {
      if (next === viewMode) {
        return;
      }
      viewMode = next;
      if (viewMode === "walk") {
        // 环绕控制器会让拖动和第一视角的拖动抢鼠标，必须停掉。
        controls.enabled = false;
        walker?.dispose();
        walker = createWalkController({
          camera: walkCamera,
          dom: renderer.domElement,
          voxels: walkVoxels,
          bounds: walkBounds,
          active: true,
        });
        activeCamera = walkCamera;
      } else {
        walker?.dispose();
        walker = null;
        controls.enabled = true;
        activeCamera = camera;
        // 第一视角在 canvas 上监听了 keydown，退出时把焦点交还，别让
        // 空格键继续被画布吞掉（否则按空格翻不了页面、也点不了按钮）。
        renderer.domElement.blur();
      }
    };

    const onRequestMode = (event: Event) => {
      const request = event as CustomEvent<ViewMode>;
      applyMode(request.detail);
    };
    container.addEventListener(VIEW_MODE_EVENT, onRequestMode);

    let frame = 0;
    let previous = performance.now();
    const animate = () => {
      frame = requestAnimationFrame(animate);
      const now = performance.now();
      const delta = (now - previous) / 1000;
      previous = now;
      if (viewMode === "walk") {
        walker?.update(delta);
      } else {
        controls.update();
      }
      renderer.render(scene, activeCamera);
    };
    animate();

    const observer = new ResizeObserver(() => {
      const width = container.clientWidth;
      const height = Math.max(1, container.clientHeight);
      // 正交相机没有 `aspect`，重算 left/right 保持比例。
      const nextAspect = width / height;
      camera.left = -half * nextAspect;
      camera.right = half * nextAspect;
      camera.top = half;
      camera.bottom = -half;
      camera.updateProjectionMatrix();
      walkCamera.aspect = nextAspect;
      walkCamera.updateProjectionMatrix();
      renderer.setSize(width, height);
    });
    observer.observe(container);

    return () => {
      container.removeEventListener(VIEW_MODE_EVENT, onRequestMode);
      observer.disconnect();
      cancelAnimationFrame(frame);
      walker?.dispose();
      controls.removeEventListener("change", applySensitivity);
      controls.dispose();
      for (const item of disposables) {
        item.dispose();
      }
      renderer.dispose();
      if (renderer.domElement.parentNode === container) {
        container.removeChild(renderer.domElement);
      }
    };
  }, [detail, pack]);

  if (failed) {
    return (
      <div className="schematic-preview-fallback" role="alert">
        当前的浏览器环境不支持 WebGL，无法显示 3D 预览。
      </div>
    );
  }

  if (detail.regions.every((region) => region.indices.length === 0)) {
    return (
      <div className="schematic-preview-fallback">
        这个投影没有可显示的方块。
      </div>
    );
  }

  return (
    <div className="schematic-preview-shell">
      <div
        ref={containerRef}
        className="schematic-preview"
        role="img"
        aria-label={`投影预览：${detail.metadata.name}`}
      />
      {pack ? (
        <div className="preview-mode-switch">
          <button
            type="button"
            className={mode === "orbit" ? "mode-active" : undefined}
            onClick={() => setMode("orbit")}
          >
            环绕视角
          </button>
          <button
            type="button"
            className={mode === "walk" ? "mode-active" : undefined}
            onClick={() => setMode("walk")}
          >
            第一视角
          </button>
        </div>
      ) : null}
      {pack && mode === "walk" ? (
        <div className="preview-keys">
          WASD 移动 · 空格上升 · Shift 下降 · 双击 W 冲刺 · 双击空格切飞行 · 拖动鼠标转视角
        </div>
      ) : null}
    </div>
  );
}

type Disposable = { dispose: () => void };

/**
 * 贴图路径：三个渲染层各一个 `Mesh`，共用同一张图集纹理。
 *
 * 材质用 `MeshBasicMaterial`：亮度已经算进顶点色（`voxelLight`），再叠一层
 * three.js 的光照只会让画面跟着相机漂。`cutout` 用 `alphaTest` 做二值透明
 * （树叶、铁栏杆），`translucent` 用普通混合并把 `depthWrite` 关掉，避免玻璃
 * 后面的东西被挡住。
 */
function addTexturedMeshes(
  scene: THREE.Scene,
  geometry: ReturnType<typeof buildTexturedGeometry>,
  pack: AssetPack,
  disposables: Disposable[],
): void {
  const texture = new THREE.TextureLoader().load(pack.atlasDataUrl, (loaded) => {
    // 图集是像素风，必须最近邻放大，否则方块纹理会被插值糊成一团。
    loaded.magFilter = THREE.NearestFilter;
    loaded.minFilter = THREE.NearestFilter;
    loaded.generateMipmaps = false;
    loaded.colorSpace = THREE.SRGBColorSpace;
    // **必须关掉 flipY**：three.js 默认在上传时把图片上下翻转，让 UV 原点落在
    // 左下；而 Minecraft 的 UV 是「v 越大越靠下」，与 PNG 的行序一致。两者
    // 叠加会让每个面的贴图上下颠倒。
    loaded.flipY = false;
    loaded.needsUpdate = true;
  });
  // 加载完成前也要先设好，避免首帧按默认值上传。
  texture.flipY = false;
  disposables.push(texture);

  const layers: [LayerBuffers, THREE.MeshBasicMaterial][] = [
    [
      geometry.opaque,
      new THREE.MeshBasicMaterial({
        map: texture,
        side: THREE.FrontSide,
        vertexColors: true,
      }),
    ],
    [
      geometry.cutout,
      new THREE.MeshBasicMaterial({
        map: texture,
        // 必须只画正面，和原版一致。原版模型里有大量「位置重合、法线相反」
        // 的一对面：十字花草的 north/south、相邻两片树叶或两块活板门的交界面。
        // 只画正面时每个方向只看得到其中一面；`DoubleSide` 会让两面深度完全
        // 相同地抢像素，而两面 UV 互为镜像，一旋转就闪。
        side: THREE.FrontSide,
        alphaTest: 0.5,
        transparent: false,
        vertexColors: true,
      }),
    ],
    [
      geometry.translucent,
      new THREE.MeshBasicMaterial({
        map: texture,
        // **只画正面**。原来用 `DoubleSide` 是想着"玻璃能看穿到背面"，但
        // 一个方块的正反两面深度完全相等，双面必然 z-fighting —— 视角一动就闪。
        // 同组剔除已经去掉了内部重叠面，正面足够。
        side: THREE.FrontSide,
        transparent: true,
        // 不额外压透明度：贴图自带 alpha（水约 180/255、彩色玻璃约 102/255）。
        opacity: 1,
        depthWrite: false,
        vertexColors: true,
      }),
    ],
  ];

  for (const [buffers, material] of layers) {
    if (buffers.count === 0) {
      material.dispose();
      continue;
    }
    const buffer = new THREE.BufferGeometry();
    buffer.setAttribute("position", new THREE.BufferAttribute(buffers.positions, 3));
    buffer.setAttribute("normal", new THREE.BufferAttribute(buffers.normals, 3));
    buffer.setAttribute("uv", new THREE.BufferAttribute(buffers.uvs, 2));
    buffer.setAttribute("color", new THREE.BufferAttribute(buffers.tints, 3));
    buffer.computeBoundingSphere();
    scene.add(new THREE.Mesh(buffer, material));
    disposables.push(buffer, material);
  }
}

function addGrid(
  scene: THREE.Scene,
  bounds: SceneBounds,
  disposables: Disposable[],
): void {
  const size = Math.ceil(bounds.radius * 2.4);
  const grid = new THREE.GridHelper(
    size,
    Math.max(1, Math.round(size)),
    0x3a3f47,
    0x262a30,
  );
  // 网格铺在建筑底面，对准包围盒中心（投影图坐标不以原点为中心）。
  grid.position.set(
    bounds.center.x,
    bounds.center.y - bounds.radius,
    bounds.center.z,
  );
  scene.add(grid);
  disposables.push(grid);
}

/** 场景包围盒：中心点与半径。 */
type SceneBounds = {
  center: THREE.Vector3;
  /** 从中心到最远顶点的距离，用来定正交相机的视野与远平面。 */
  radius: number;
};

/**
 * 从一组顶点里量出真实的包围盒。
 *
 * **必须用 min/max 求中心**，不能只取绝对值最大：投影图的方块用世界绝对
 * 坐标（y 可能从 60 开始、z 从 183 开始），只取 |坐标| 会把原点也算进去，
 * 中心算错，整栋建筑就被推出视野。
 */
function boundsOfPositions(positions: ArrayLike<number>): SceneBounds {
  if (positions.length < 3) {
    return { center: new THREE.Vector3(0, 0, 0), radius: 8 };
  }
  let minX = Infinity;
  let minY = Infinity;
  let minZ = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let maxZ = -Infinity;
  for (let index = 0; index + 2 < positions.length; index += 3) {
    const x = positions[index];
    const y = positions[index + 1];
    const z = positions[index + 2];
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    minZ = Math.min(minZ, z);
    maxX = Math.max(maxX, x);
    maxY = Math.max(maxY, y);
    maxZ = Math.max(maxZ, z);
  }
  const center = new THREE.Vector3(
    (minX + maxX) / 2,
    (minY + maxY) / 2,
    (minZ + maxZ) / 2,
  );
  const radius = Math.max(maxX - minX, maxY - minY, maxZ - minZ) / 2 + 1;
  return { center, radius };
}

/** 三层几何合起来的包围盒。 */
function boundsOfLayers(
  geometry: ReturnType<typeof buildTexturedGeometry>,
): SceneBounds {
  const layers = [geometry.opaque, geometry.cutout, geometry.translucent];
  const total = layers.reduce((sum, layer) => sum + layer.positions.length, 0);
  if (total === 0) {
    return { center: new THREE.Vector3(0, 0, 0), radius: 8 };
  }
  const merged = new Float32Array(total);
  let offset = 0;
  for (const layer of layers) {
    merged.set(layer.positions, offset);
    offset += layer.positions.length;
  }
  return boundsOfPositions(merged);
}
