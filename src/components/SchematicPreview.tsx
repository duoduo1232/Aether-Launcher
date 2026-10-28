import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { AssetPack, SchematicDetail } from "../ipc";
import { buildRenderData } from "../lib/schematicRender";
import {
  buildTexturedGeometry,
  type LayerBuffers,
} from "../lib/texturedMesh";

type SchematicPreviewProps = {
  detail: SchematicDetail;
  /**
   * 从版本 jar 提取的方块资源包。为空时退回纯色方块 —— 形状仍然对，但看不到
   * 木板纹理、半砖斜面这些细节。
   */
  pack?: AssetPack | null;
};

/**
 * 投影图的 3D 预览。
 *
 * 两条渲染路径：
 * - **有资源包**：按真实方块模型展开成带 UV 的三角形，一张图集贴图，按
 *   `opaque` / `cutout` / `translucent` 分三批绘制。楼梯、半砖、栏杆的形状和
 *   纹理都来自 Minecraft 自己的模型定义。
 * - **没有资源包**：退回「一个方块一个实例 + 代表色」（`InstancedMesh`），
 *   形状全是整方块，但至少能看出体量。
 *
 * 视角固定为正交等轴测（`OrthographicCamera`）：投影图是方格建筑，透视会让
 * 远处缩小、看起来是歪的。支持拖动旋转、滚轮缩放、右键平移。
 */
export function SchematicPreview({ detail, pack = null }: SchematicPreviewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    // 有资源包走贴图；没有就走纯色。两条路径都要先算出场景尺度。
    const textured = pack ? buildTexturedGeometry(detail, pack) : null;
    const solid = pack ? null : buildRenderData(detail);
    const hasContent = textured
      ? textured.quadCount > 0 || textured.fallback.count > 0
      : (solid?.count ?? 0) > 0;
    if (!hasContent) {
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
    // 透明方块要按像素混合，所以关掉自动清屏的深度写入顺序假设。
    renderer.setClearColor(0x1b1d23, 1);
    container.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x1b1d23);

    // 场景尺度：贴图路径用顶点包围盒，纯色路径沿用体素包围盒。
    const extent = textured
      ? extentOfLayers(textured)
      : estimateExtent(solid!);

    const aspect = container.clientWidth / Math.max(1, container.clientHeight);
    const half = extent * 1.15;
    const camera = new THREE.OrthographicCamera(
      -half * aspect,
      half * aspect,
      half,
      -half,
      0.1,
      Math.max(200, extent * 40),
    );
    // 等轴测：从 (±1, ±0.8, ±1) 方向看。
    camera.position.set(extent * 1.5, extent * 1.2, extent * 1.5);
    camera.lookAt(0, 0, 0);

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.08;
    controls.target.set(0, 0, 0);
    controls.update();

    // 模拟 Minecraft 的光照：环境光很暗，主要靠两盏方向光，方块之间才有层次。
    scene.add(new THREE.AmbientLight(0xffffff, 0.55));
    const sun = new THREE.DirectionalLight(0xffffff, 1.35);
    sun.position.set(extent, extent * 2, extent);
    scene.add(sun);
    const fill = new THREE.DirectionalLight(0xffffff, 0.45);
    fill.position.set(-extent, extent, -extent);
    scene.add(fill);

    const disposables: Disposable[] = [];

    if (textured) {
      addTexturedMeshes(scene, textured, pack!, disposables);
      if (textured.fallback.count > 0) {
        addFallbackMeshes(scene, detail, textured.fallback.count, disposables);
      }
      addGrid(scene, extent, disposables);
    } else {
      addSolidMesh(scene, solid!, disposables);
      addGrid(scene, extent, disposables);
    }

    let frame = 0;
    const animate = () => {
      frame = requestAnimationFrame(animate);
      controls.update();
      renderer.render(scene, camera);
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
      renderer.setSize(width, height);
    });
    observer.observe(container);

    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
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
    <div
      ref={containerRef}
      className="schematic-preview"
      role="img"
      aria-label={`投影预览：${detail.metadata.name}`}
    />
  );
}

type Disposable = { dispose: () => void };

/**
 * 贴图路径：三个渲染层各一个 `Mesh`，共用同一张图集纹理。
 *
 * `cutout` 用 `alphaTest` 做二值透明（树叶、铁栏杆），`translucent` 用普通
 * 混合并把 `depthWrite` 关掉，避免玻璃后面的东西被挡住。
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
    loaded.needsUpdate = true;
  });
  disposables.push(texture);

  const layers: [LayerBuffers, THREE.MeshLambertMaterial][] = [
    [
      geometry.opaque,
      new THREE.MeshLambertMaterial({ map: texture, side: THREE.FrontSide }),
    ],
    [
      geometry.cutout,
      new THREE.MeshLambertMaterial({
        map: texture,
        side: THREE.DoubleSide,
        alphaTest: 0.5,
        transparent: false,
      }),
    ],
    [
      geometry.translucent,
      new THREE.MeshLambertMaterial({
        map: texture,
        side: THREE.DoubleSide,
        transparent: true,
        opacity: 0.75,
        depthWrite: false,
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
    buffer.computeBoundingSphere();
    scene.add(new THREE.Mesh(buffer, material));
    disposables.push(buffer, material);
  }
}

/** 资源包里没有的方块：退回纯色整方块，至少不让它们消失。 */
function addFallbackMeshes(
  scene: THREE.Scene,
  detail: SchematicDetail,
  expected: number,
  disposables: Disposable[],
): void {
  const data = buildRenderData(detail, Number.MAX_SAFE_INTEGER);
  if (data.count === 0) {
    return;
  }
  const geometry = new THREE.BoxGeometry(1, 1, 1);
  const material = new THREE.MeshLambertMaterial();
  const mesh = new THREE.InstancedMesh(geometry, material, data.count);
  const matrix = new THREE.Matrix4();
  const color = new THREE.Color();
  const position = new THREE.Vector3();
  const scale = new THREE.Vector3(1, 1, 1);
  const quaternion = new THREE.Quaternion();
  for (let index = 0; index < data.count; index += 1) {
    position.set(
      data.positions[index * 3],
      data.positions[index * 3 + 1],
      data.positions[index * 3 + 2],
    );
    matrix.compose(position, quaternion, scale);
    mesh.setMatrixAt(index, matrix);
    color.setRGB(
      data.colors[index * 3],
      data.colors[index * 3 + 1],
      data.colors[index * 3 + 2],
      THREE.SRGBColorSpace,
    );
    mesh.setColorAt(index, color);
  }
  mesh.instanceMatrix.needsUpdate = true;
  if (mesh.instanceColor) {
    mesh.instanceColor.needsUpdate = true;
  }
  mesh.visible = expected > 0;
  scene.add(mesh);
  disposables.push(geometry, material, mesh);
}

/** 没有资源包时的纯色路径。 */
function addSolidMesh(
  scene: THREE.Scene,
  data: ReturnType<typeof buildRenderData>,
  disposables: Disposable[],
): void {
  const geometry = new THREE.BoxGeometry(1, 1, 1);
  const material = new THREE.MeshLambertMaterial();
  const mesh = new THREE.InstancedMesh(geometry, material, data.count);
  const matrix = new THREE.Matrix4();
  const color = new THREE.Color();
  const position = new THREE.Vector3();
  const scale = new THREE.Vector3();
  const quaternion = new THREE.Quaternion();
  for (let index = 0; index < data.count; index += 1) {
    position.set(
      data.positions[index * 3],
      data.positions[index * 3 + 1],
      data.positions[index * 3 + 2],
    );
    scale.setScalar(data.stride * 0.95);
    matrix.compose(position, quaternion, scale);
    mesh.setMatrixAt(index, matrix);
    color.setRGB(
      data.colors[index * 3],
      data.colors[index * 3 + 1],
      data.colors[index * 3 + 2],
      THREE.SRGBColorSpace,
    );
    mesh.setColorAt(index, color);
  }
  mesh.instanceMatrix.needsUpdate = true;
  if (mesh.instanceColor) {
    mesh.instanceColor.needsUpdate = true;
  }
  scene.add(mesh);
  disposables.push(geometry, material, mesh);
}

function addGrid(scene: THREE.Scene, extent: number, disposables: Disposable[]): void {
  const size = Math.ceil(extent * 1.5);
  const grid = new THREE.GridHelper(size, Math.max(1, Math.round(size)), 0x3a3f47, 0x262a30);
  grid.position.y = -extent / 2 - 0.5;
  scene.add(grid);
  disposables.push(grid);
}

/** 三层的顶点包围盒里最大的半边长。 */
function extentOfLayers(
  geometry: ReturnType<typeof buildTexturedGeometry>,
): number {
  let maxAbs = 1;
  for (const layer of [geometry.opaque, geometry.cutout, geometry.translucent]) {
    const positions = layer.positions;
    for (let index = 0; index < positions.length; index += 1) {
      const value = Math.abs(positions[index]);
      if (value > maxAbs) {
        maxAbs = value;
      }
    }
  }
  return maxAbs * 1.3 + 4;
}

/** 用体素分布的包围盒估算场景尺度。 */
function estimateExtent(data: ReturnType<typeof buildRenderData>): number {
  let maxAbs = 1;
  for (let index = 0; index < data.positions.length; index += 1) {
    const abs = Math.abs(data.positions[index]);
    if (abs > maxAbs) {
      maxAbs = abs;
    }
  }
  return maxAbs * 1.4 + 4;
}
