import type { SchematicSummary } from "../ipc";

/**
 * 把后端返回的扁平投影列表还原成 `schematics/` 的目录树。
 *
 * 后端只给 `relativePath`（`/` 分隔），目录结构由这里重建：文件夹在前、
 * 文件在后，各自按名字排序。`total` 是该文件夹（含所有子文件夹）里的投影
 * 数量，用来在折叠状态下显示计数。
 */
export type SchematicFolderNode = {
  kind: "folder";
  /** `schematics/` 下的相对路径；根节点是空字符串。 */
  path: string;
  name: string;
  folders: SchematicFolderNode[];
  files: SchematicSummary[];
  /** 这个文件夹（含子文件夹）里的投影总数。 */
  total: number;
};

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
