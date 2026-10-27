import { callCommand } from "./call";
import { isTauri } from "./index";
import type { InstanceState } from "./instances";

/**
 * 投影图（`.litematic`）命令的类型化封装。
 *
 * 命令定义在 `src-tauri/src/commands/schematics.rs`：投影文件**自动按实例
 * 识别**——后端递归扫描每个实例游戏目录下的 `schematics/`，子文件夹结构
 * 用 `relativePath`（`/` 分隔）带回来。IPC 只传实例 ID、`schematics/` 下的
 * 相对路径与文件内容，从不传绝对路径。
 *
 * 列表只包含**有投影的实例**：没有 `schematics/` 目录的版本不会出现。
 */

declare module "./call" {
  interface CommandArgs {
    list_schematics: void;
    read_schematic: { instanceId: string; relativePath: string };
    import_schematic: { instanceId: string; fileName: string; bytes: number[] };
    import_schematic_folder: { instanceId: string };
    delete_schematic: { instanceId: string; relativePath: string };
    paste_schematic: {
      fromInstanceId: string;
      fromPath: string;
      toInstanceId: string;
      toFolder: string;
      moveFile: boolean;
    };
  }

  interface CommandResults {
    list_schematics: SchematicListing;
    read_schematic: SchematicDetail;
    import_schematic: string;
    import_schematic_folder: SchematicFolderImport | null;
    delete_schematic: null;
    paste_schematic: string;
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
  /** 相对**所属来源目录**的路径，用 `/` 分隔（如 `城堡/主楼.litematic`）。 */
  relativePath: string;
  fileName: string;
  sizeBytes: number;
  /** 这份投影在哪个 `schematics/` 里：实例自己的 / 游戏目录根上的 / 某个版本的。 */
  origin: SchematicOrigin;
  /** 来源所在的版本目录名；`instance` 和 `game` 为空字符串。 */
  originVersion: string;
  metadata: SchematicMetadata;
};

/** 扫描时解析失败的文件；reason 是稳定分类（unreadable/unparseable/oversized）。 */
export type FailedSchematic = {
  relativePath: string;
  fileName: string;
  reason: string;
  origin: SchematicOrigin;
  originVersion: string;
};

/**
 * 投影所在的位置。`.minecraft` 里投影是多个版本共用的，所以有三处：
 * 实例自己的目录、游戏目录根上的 `schematics/`、`versions/<版本>/schematics`。
 */
export type SchematicOrigin = "instance" | "game" | "version";

/** 实际扫到内容的来源目录，界面用来说明「这些投影从哪读到的」。 */
export type SchematicSource = {
  origin: SchematicOrigin;
  originVersion: string;
  /** 相对描述，如 `schematics` 或 `versions/1.20.1/schematics`。 */
  label: string;
};

/** 一个有投影的实例自动识别出的全部投影图。 */
export type InstanceSchematics = {
  id: string;
  name: string;
  version: string;
  state: InstanceState;
  schematics: SchematicSummary[];
  failed: FailedSchematic[];
  /** 实际扫到内容的来源目录。 */
  sources: SchematicSource[];
};

export type SchematicListing = {
  /** 只包含有投影（或解析失败文件）的实例。 */
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
  relativePath: string;
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
  relativePath: string,
): Promise<SchematicDetail> {
  return callCommand("read_schematic", { instanceId, relativePath });
}

/** 把文件内容导入指定实例的 `schematics/` 根目录。浏览器预览会 reject。 */
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

/** 一个被文件夹导入跳过的文件；`reason` 是稳定分类。 */
export type SchematicFolderSkip = {
  relativePath: string;
  reason: string;
};

/** 一次文件夹导入的结果。 */
export type SchematicFolderImport = {
  imported: number;
  skipped: SchematicFolderSkip[];
};

/**
 * 把一个文件夹里的 `.litematic` 全部导入指定实例，保留子文件夹结构。
 *
 * 文件夹由**后端**用系统选择框打开，前端只给实例 ID；取消选择时返回 `null`。
 */
export async function importSchematicFolder(
  instanceId: string,
): Promise<SchematicFolderImport | null> {
  return callCommand("import_schematic_folder", { instanceId });
}

export async function deleteSchematic(
  instanceId: string,
  relativePath: string,
): Promise<void> {
  await callCommand("delete_schematic", { instanceId, relativePath });
}

/**
 * 把投影复制或移动到目标文件夹，返回新的相对路径。
 *
 * 源和目标可以是同一个实例（整理目录），也可以是不同实例（跨版本共享）。
 * `toFolder` 传空字符串表示 `schematics/` 根目录；目标已有同名文件时后端
 * 自动改名成 `名字 (2).litematic`，不覆盖已有投影。
 */
export async function pasteSchematic(
  fromInstanceId: string,
  fromPath: string,
  toInstanceId: string,
  toFolder: string,
  moveFile: boolean,
): Promise<string> {
  return callCommand("paste_schematic", {
    fromInstanceId,
    fromPath,
    toInstanceId,
    toFolder,
    moveFile,
  });
}
