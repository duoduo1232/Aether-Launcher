import type { SchematicOrigin, SchematicSummary } from "../ipc";

/**
 * 把后端返回的扁平投影列表还原成目录树。
 *
 * 投影可能来自三处（实例自己的目录、游戏目录根上的 `schematics/`、某个版本的
 * `schematics/`），同一份相对路径在三个来源里都可能存在，所以树按来源分区：
 * 每棵子树只装一个来源的文件，`relativePath` 在子树内唯一，IPC 用它定位文件
 * 就够了。
 *
 * 文件夹在前、文件在后，各自按名字排序。`total` 是该文件夹（含所有子文件夹）
 * 里的投影数量，用来在折叠状态下显示计数。
 */
export type SchematicFolderNode = {
  kind: "folder";
  /** 相对**所属来源目录**的路径；根节点是空字符串。 */
  path: string;
  name: string;
  folders: SchematicFolderNode[];
  files: SchematicSummary[];
  /** 这个文件夹（含子文件夹）里的投影总数。 */
  total: number;
};

/** 一个来源及其目录树。 */
export type SchematicSourceTree = {
  origin: SchematicOrigin;
  /** 版本目录名；`instance` 和 `game` 为空字符串。 */
  originVersion: string;
  /** 界面标题，如 `schematics` 或 `versions/1.20.1/schematics`。 */
  label: string;
  root: SchematicFolderNode;
  /** 这个来源里的投影总数。 */
  total: number;
};

/** 来源的展示名；和后端 `source_label` 保持一致。 */
export function sourceLabel(origin: SchematicOrigin, originVersion: string): string {
  switch (origin) {
    case "version":
      return `versions/${originVersion}/schematics`;
    case "game":
      return "schematics";
    default:
      return "实例目录/schematics";
  }
}

function sourceKey(file: SchematicSummary): string {
  return `${file.origin}|${file.originVersion}`;
}

/**
 * 按来源分区，每个来源各自建一棵树。
 *
 * 只有一个来源时返回一棵树，界面不用显示来源标题；有多个来源时每个来源一个
 * 节点，避免不同 `schematics/` 里同名同路径的文件互相覆盖。
 */
export function buildSchematicSources(
  files: SchematicSummary[],
): SchematicSourceTree[] {
  const groups = new Map<string, { origin: SchematicOrigin; originVersion: string; files: SchematicSummary[] }>();
  for (const file of files) {
    const key = sourceKey(file);
    let group = groups.get(key);
    if (!group) {
      group = { origin: file.origin, originVersion: file.originVersion, files: [] };
      groups.set(key, group);
    }
    group.files.push(file);
  }

  return [...groups.values()].map((group) => {
    const root = buildSchematicTree(group.files);
    return {
      origin: group.origin,
      originVersion: group.originVersion,
      label: sourceLabel(group.origin, group.originVersion),
      root,
      total: root.total,
    };
  });
}

/** 建一棵树；同一来源内的 `relativePath` 必须唯一。 */
export function buildSchematicTree(files: SchematicSummary[]): SchematicFolderNode {
  const root = createFolder("", "");
  for (const file of files) {
    const segments = file.relativePath.split("/").filter((segment) => segment.length > 0);
    // 最后一段是文件名，前面的是文件夹。
    segments.pop();
    let node = root;
    let prefix = "";
    for (const segment of segments) {
      prefix = prefix ? `${prefix}/${segment}` : segment;
      let child = node.folders.find((folder) => folder.path === prefix);
      if (!child) {
        child = createFolder(prefix, segment);
        node.folders.push(child);
      }
      node = child;
    }
    node.files.push(file);
  }
  sortNode(root);
  return root;
}

function createFolder(path: string, name: string): SchematicFolderNode {
  return { kind: "folder", path, name, folders: [], files: [], total: 0 };
}

function sortNode(node: SchematicFolderNode): number {
  node.folders.sort((left, right) => left.name.localeCompare(right.name));
  node.files.sort((left, right) => left.fileName.localeCompare(right.fileName));
  let total = node.files.length;
  for (const folder of node.folders) {
    total += sortNode(folder);
  }
  node.total = total;
  return total;
}

/** 列出这个文件夹（含子文件夹）里的全部投影相对路径。 */
export function collectFilePaths(node: SchematicFolderNode): string[] {
  const paths = node.files.map((file) => file.relativePath);
  for (const folder of node.folders) {
    paths.push(...collectFilePaths(folder));
  }
  return paths;
}

/**
 * 列出这个文件夹里的全部投影文件（含子文件夹）。
 * 导入、删除、粘贴都只作用在实例自己的 `schematics/` 里，调用方据此过滤来源。
 */
export function collectFiles(node: SchematicFolderNode): SchematicSummary[] {
  const files = [...node.files];
  for (const folder of node.folders) {
    files.push(...collectFiles(folder));
  }
  return files;
}
