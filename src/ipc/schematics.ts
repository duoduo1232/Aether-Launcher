import { callCommand } from "./call";
import { isTauri } from "./index";
import type { InstanceState } from "./instances";

/**
 * 投影图（`.litematic`）命令的类型化封装。
 *
 * 命令定义在 `src-tauri/src/commands/schematics.rs`：投影文件**自动按实例
 * 识别**——后端扫描每个实例游戏目录下的 `schematics/`。IPC 只传实例 ID、
 * 裸文件名与文件内容，从不传路径。
 */

declare module "./call" {
  interface CommandArgs {
    list_schematics: void;
    read_schematic: { instanceId: string; fileName: string };
    import_schematic: { instanceId: string; fileName: string; bytes: number[] };
    delete_schematic: { instanceId: string; fileName: string };
    copy_schematic_to_instance: {
      fromInstanceId: string;
      toInstanceId: string;
      fileName: string;
    };
  }

  interface CommandResults {
    list_schematics: SchematicListing;
    read_schematic: SchematicDetail;
    import_schematic: string;
    delete_schematic: null;
    copy_schematic_to_instance: string;
  }
}

/** 投影图元数据；`createdForRelease` 是数据版本对应的正式版（跨版本提示用）。 */
export type SchematicMetadata = {
  name: string;
  author: string;
  description: string;
  formatVersion: number;
  minecraftDataVersion: number;
  createdForRelease: string | null;
  regionCount: number;
  totalBlocks: number;
  totalVolume: number;
  enclosingSize: [number, number, number];
};

export type SchematicSummary = {
  fileName: string;
  sizeBytes: number;
  metadata: SchematicMetadata;
};

/** 扫描时解析失败的文件；reason 是稳定分类（unreadable/unparseable/oversized）。 */
export type FailedSchematic = {
  fileName: string;
  reason: string;
};

/** 一个实例自动识别出的全部投影图。 */
export type InstanceSchematics = {
  id: string;
  name: string;
  version: string;
  state: InstanceState;
  schematics: SchematicSummary[];
  failed: FailedSchematic[];
};

export type SchematicListing = {
  instances: InstanceSchematics[];
  skipped: string[];
};

/** 一个区域的解码结果；indices 按 (y·sizeZ + z)·sizeX + x 排列。 */
export type SchematicRegion = {
  name: string;
  position: [number, number, number];
  size: [number, number, number];
  palette: string[];
  indices: number[];
};

export type SchematicDetail = {
  instanceId: string;
  fileName: string;
  metadata: SchematicMetadata;
  regions: SchematicRegion[];
};

export const emptySchematicListing: SchematicListing = {
  instances: [],
  skipped: [],
};

/** 与后端一致的导入大小上限：16 MiB（更大的文件直接放进实例的目录即可）。 */
export const IMPORT_LIMIT_BYTES = 16 * 1024 * 1024;

export async function listSchematics(): Promise<SchematicListing> {
  if (!isTauri()) {
    return emptySchematicListing;
  }
  return callCommand("list_schematics");
}

export async function readSchematic(
  instanceId: string,
  fileName: string,
): Promise<SchematicDetail> {
  return callCommand("read_schematic", { instanceId, fileName });
}

/** 把文件内容导入指定实例。浏览器预览会 reject，调用方负责提示。 */
export async function importSchematic(
  instanceId: string,
  fileName: string,
  bytes: Uint8Array,
): Promise<string> {
  return callCommand("import_schematic", {
    instanceId,
    fileName,
    bytes: Array.from(bytes),
  });
}

export async function deleteSchematic(
  instanceId: string,
  fileName: string,
): Promise<void> {
  await callCommand("delete_schematic", { instanceId, fileName });
}

/** 跨实例复制（跨版本共享）；源与目标相同时后端会拒绝。 */
export async function copySchematicToInstance(
  fromInstanceId: string,
  toInstanceId: string,
  fileName: string,
): Promise<string> {
  return callCommand("copy_schematic_to_instance", {
    fromInstanceId,
    toInstanceId,
    fileName,
  });
}
