import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { SchematicDetail } from "../ipc";
import { buildRenderData } from "../lib/schematicRender";

type SchematicPreviewProps = {
  detail: SchematicDetail;
};

/**
 * 投影图的 3D 预览：每个方块一个实例（InstancedMesh），支持鼠标旋转、
 * 缩放与平移（OrbitControls）。
 *
 * WebGL 只能在浏览器里初始化，组件加载后即渲染；数据变化时整个场景重建。
 * 体素超过上限时 `buildRenderData` 会按晶格抽样，`stride` 用来把方块放大，
 * 保持形状比例。
 */
export function SchematicPreview({ detail }: SchematicPreviewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    const data = buildRenderData(detail);
    if (data.count === 0) {
      return;
    }

    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({ antialias: true });
    } catch {
      // WebGL 不可用：把失败状态放到微任务里标记，避免在 effect 里同步 setState。
      queueMicrotask(() => setFailed(true));
      return;
    }
    renderer.setPixelRatio(window.devicePixelRatio);
    renderer.setSize(container.clientWidth, container.clientHeight);
    container.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x1b1d23);

    const extent = estimateExtent(data);
    const camera = new THREE.PerspectiveCamera(
      50,
      container.clientWidth / Math.max(1, container.clientHeight),
      0.1,
      Math.max(100, extent * 10),
    );
    camera.position.set(extent, extent * 0.8, extent);

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.08;
    controls.target.set(0, 0, 0);

    scene.add(new THREE.HemisphereLight(0xffffff, 0x33383f, 2.2));
    const sun = new THREE.DirectionalLight(0xffffff, 1.6);
    sun.position.set(extent, extent * 2, extent);
    scene.add(sun);

    const gridExtent = Math.ceil(extent * 1.5);
    const grid = new THREE.GridHelper(
      gridExtent,
      Math.max(1, Math.round(gridExtent)),
      0x3a3f47,
      0x262a30,
    );
    grid.position.y = -extent / 2 - 0.5;
    scene.add(grid);

    const geometry = new THREE.BoxGeometry(1, 1, 1);
    const material = new THREE.MeshLambertMaterial();
    const mesh = new THREE.InstancedMesh(geometry, material, data.count);
    const matrix = new THREE.Matrix4();
    const quaternion = new THREE.Quaternion();
    const scale = new THREE.Vector3();
    const position = new THREE.Vector3();
    const color = new THREE.Color();
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
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
      renderer.setSize(width, height);
    });
    observer.observe(container);

    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
      controls.dispose();
      scene.remove(mesh);
      geometry.dispose();
      material.dispose();
      grid.dispose();
      renderer.dispose();
      container.removeChild(renderer.domElement);
    };
  }, [detail]);

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

  return <div ref={containerRef} className="schematic-preview" role="img" aria-label={`投影预览：${detail.metadata.name}`} />;
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
