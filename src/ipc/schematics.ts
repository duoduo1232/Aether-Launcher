import { callCommand } from "./call";
import { isTauri } from "./index";

/**
 * 投影图（`.litematic`）命令的类型化封装。
 *
 * 命令定义在 `src-tauri/src/commands/schematics.rs`。文件统一放在应用私有
 * 的 `schematics` 目录：IPC 只传文件名与文件内容，从不传路径。导入用
 * `import_schematic` 传字节，删除与复制只传名字。
 */

declare module "./call" {
  interface CommandArgs {
    list_schematics: void;
    read_schematic: { fileName: string };
    import_schematic: { fileName: string; bytes: number[] };
    delete_schematic: { fileName: string };
    copy_schematic_to_instance: { fileName: string; instanceId: string };
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

/** 列表里解析失败的文件；reason 是稳定分类（unreadable/unparseable/oversized）。 */
export type FailedSchematic = {
  fileName: string;
  reason: string;
};

export type SchematicListing = {
  schematics: SchematicSummary[];
  failed: FailedSchematic[];
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
  fileName: string;
  metadata: SchematicMetadata;
  regions: SchematicRegion[];
};

export const emptySchematicListing: SchematicListing = {
  schematics: [],
  failed: [],
};

/** 与后端一致的导入大小上限：16 MiB（更大的文件放进受管理目录即可）。 */
export const IMPORT_LIMIT_BYTES = 16 * 1024 * 1024;

export async function listSchematics(): Promise<SchematicListing> {
  if (!isTauri()) {
    return emptySchematicListing;
  }
  return callCommand("list_schematics");
}

export async function readSchematic(fileName: string): Promise<SchematicDetail> {
  return callCommand("read_schematic", { fileName });
}

/** 导入一个文件的内容。浏览器预览会 reject，调用方负责提示。 */
export async function importSchematic(
  fileName: string,
  bytes: Uint8Array,
): Promise<string> {
  return callCommand("import_schematic", {
    fileName,
    bytes: Array.from(bytes),
  });
}

export async function deleteSchematic(fileName: string): Promise<void> {
  await callCommand("delete_schematic", { fileName });
}

export async function copySchematicToInstance(
  fileName: string,
  instanceId: string,
): Promise<string> {
  return callCommand("copy_schematic_to_instance", { fileName, instanceId });
}
