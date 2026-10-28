import {
  Badge,
  Button,
  Menu,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
  Text,
} from "@fluentui/react-components";
import {
  CheckboxChecked24Regular,
  ChevronDown24Regular,
  ChevronRight24Regular,
  ClipboardPaste24Regular,
  Copy24Regular,
  Cut24Regular,
  Delete24Regular,
  Document24Regular,
  Folder24Regular,
  FolderOpen24Regular,
} from "@fluentui/react-icons";
import { useState } from "react";
import type { SchematicSummary } from "../ipc";
import type { SchematicFolderNode } from "../lib/schematicTree";
import { collectFiles } from "../lib/schematicTree";

/// 根目录首次渲染的文件数上限；每次点「继续显示」再加这么多。
const ROOT_FILE_PAGE = 100;

/**
 * 一个版本的投影目录树。
 *
 * 文件夹结构来自后端返回的 `relativePath`：文件夹可折叠，文件行右键出菜单
 * （复制 / 剪切 / 删除 / 多选），文件夹行右键可以「粘贴到此处」。多选模式下
 * 每行前面出现复选框，右键操作作用于整个选择集。
 */
export type SchematicTreeProps = {
  root: SchematicFolderNode;
  busy: boolean;
  selectionMode: boolean;
  /** 已选中的相对路径（只属于当前实例）。 */
  selected: ReadonlySet<string>;
  /** 已选中的条目本身；批量操作要用它们各自的来源。 */
  selectedItems: SchematicSummary[];
  /**
   * 被用户**展开**的文件夹路径；默认全部折叠。
   *
   * 一个版本的 `schematics/` 根上能有七八百个文件，默认全展开就是一次渲染
   * 上千个 `Menu` 组件，页面会直接卡住。
   */
  expandedFolders: ReadonlySet<string>;
  /** 剪贴板里有内容，可以粘贴。 */
  clipboardReady: boolean;
  onToggleFolder: (path: string) => void;
  onToggleSelect: (item: SchematicSummary) => void;
  onPreview: (item: SchematicSummary) => void;
  onCopy: (items: SchematicSummary[], move: boolean) => void;
  onPaste: (folder: string) => void;
  onDelete: (items: SchematicSummary[]) => void;
  onStartSelection: (item: SchematicSummary) => void;
  onSelectAll: (items: SchematicSummary[]) => void;
};

export function SchematicTree(props: SchematicTreeProps) {
  return (
    <div className="schematic-tree">
      <FolderLevel node={props.root} depth={0} {...props} />
    </div>
  );
}

type FolderLevelProps = SchematicTreeProps & {
  node: SchematicFolderNode;
  depth: number;
};

function FolderLevel({ node, depth, ...props }: FolderLevelProps) {
  // 根层可能直接挂着上千个文件（生电存档的 schematics 根上就有 778 个）。
  // 一次渲染一千个 `Menu` 会卡住主线程，所以先只渲染前一批，其余按需加载。
  const [visible, setVisible] = useState(ROOT_FILE_PAGE);
  const limited = depth === 0 && node.files.length > visible;
  const files = limited ? node.files.slice(0, visible) : node.files;
  return (
    <ul className="schematic-branch">
      {node.folders.map((folder) => (
        <FolderBranch key={folder.path} node={folder} depth={depth} {...props} />
      ))}
      {files.map((file) => (
        <FileRow key={file.relativePath} item={file} depth={depth} {...props} />
      ))}
      {limited ? (
        <li className="schematic-branch-item">
          <Button
            size="small"
            appearance="subtle"
            className="schematic-more"
            onClick={() => setVisible((count) => count + ROOT_FILE_PAGE)}
          >
            还有 {node.files.length - visible} 个文件，点这里继续显示
          </Button>
        </li>
      ) : null}
    </ul>
  );
}
type FolderBranchProps = SchematicTreeProps & {
  node: SchematicFolderNode;
  depth: number;
};

function FolderBranch({ node, depth, ...props }: FolderBranchProps) {
  // 默认折叠：展开是用户显式动作，只记展开的那些。
  const open = props.expandedFolders.has(node.path);
  const files = collectFiles(node);
  return (
    <li className="schematic-branch-item">
      <Menu openOnContext>
        <MenuTrigger disableButtonEnhancement>
          <button
            type="button"
            className="schematic-folder"
            style={{ paddingInlineStart: 8 + depth * 16 }}
            aria-expanded={open}
            aria-label={`文件夹 ${node.name}`}
            onClick={() => props.onToggleFolder(node.path)}
          >
            {open ? <ChevronDown24Regular /> : <ChevronRight24Regular />}
            {open ? <FolderOpen24Regular /> : <Folder24Regular />}
            <span className="schematic-folder-name">{node.name}</span>
            <Badge appearance="tint" size="small">
              {node.total}
            </Badge>
          </button>
        </MenuTrigger>
        <MenuPopover>
          <MenuList>
            <MenuItem
              icon={<ClipboardPaste24Regular />}
              disabled={!props.clipboardReady || props.busy}
              onClick={() => props.onPaste(node.path)}
            >
              粘贴到此处
            </MenuItem>
            <MenuItem
              icon={<CheckboxChecked24Regular />}
              disabled={props.busy}
              onClick={() => props.onSelectAll(files)}
            >
              全选此文件夹（{files.length}）
            </MenuItem>
          </MenuList>
        </MenuPopover>
      </Menu>
      {open ? <FolderLevel node={node} depth={depth + 1} {...props} /> : null}
    </li>
  );
}

type FileRowProps = SchematicTreeProps & {
  item: SchematicSummary;
  depth: number;
};

function FileRow({ item, depth, ...props }: FileRowProps) {
  const checked = props.selected.has(item.relativePath);
  // 右键作用于整个选择集，除非点的文件不在选择集里。
  const targets =
    props.selectionMode && checked && props.selectedItems.length > 0
      ? props.selectedItems
      : [item];
  return (
    <li className="schematic-branch-item">
      <Menu openOnContext>
        <MenuTrigger disableButtonEnhancement>
          <div
            className={`schematic-row${checked ? " is-selected" : ""}`}
            style={{ paddingInlineStart: 8 + depth * 16 }}
            role="button"
            tabIndex={0}
            aria-label={item.metadata.name || item.fileName}
            onClick={() =>
              props.selectionMode
                ? props.onToggleSelect(item)
                : props.onPreview(item)
            }
            onKeyDown={(event) => {
              if (event.key !== "Enter" && event.key !== " ") {
                return;
              }
              event.preventDefault();
              if (props.selectionMode) {
                props.onToggleSelect(item);
              } else {
                props.onPreview(item);
              }
            }}
          >
            {props.selectionMode ? (
              <input
                type="checkbox"
                className="schematic-check"
                checked={checked}
                aria-label={`选择 ${item.fileName}`}
                onChange={() => props.onToggleSelect(item)}
                onClick={(event) => event.stopPropagation()}
              />
            ) : (
              <Document24Regular />
            )}
            <div className="schematic-info">
              <Text weight="semibold">{item.metadata.name || item.fileName}</Text>
              <Text className="muted" size={200}>
                {item.fileName} · {formatBytes(item.sizeBytes)} ·{" "}
                {item.metadata.totalVolume.toLocaleString("zh-CN")} 格 ·{" "}
                {describeOrigin(item)}
              </Text>
            </div>
          </div>
        </MenuTrigger>
        <MenuPopover>
          <MenuList>
            <MenuItem
              icon={<Copy24Regular />}
              disabled={props.busy}
              onClick={() => props.onCopy(targets, false)}
            >
              复制{targets.length > 1 ? ` ${targets.length} 项` : ""}
            </MenuItem>
            <MenuItem
              icon={<Cut24Regular />}
              disabled={props.busy}
              onClick={() => props.onCopy(targets, true)}
            >
              剪切{targets.length > 1 ? ` ${targets.length} 项` : ""}
            </MenuItem>
            <MenuItem
              icon={<Delete24Regular />}
              disabled={props.busy}
              onClick={() => props.onDelete(targets)}
            >
              删除{targets.length > 1 ? ` ${targets.length} 项` : ""}
            </MenuItem>
            <MenuItem
              icon={<CheckboxChecked24Regular />}
              disabled={props.busy}
              onClick={() => props.onStartSelection(item)}
            >
              多选
            </MenuItem>
          </MenuList>
        </MenuPopover>
      </Menu>
    </li>
  );
}

function describeOrigin(item: SchematicSummary): string {
  const release = item.metadata.createdForRelease;
  return release ? `MC ${release}` : `数据版本 ${item.metadata.minecraftDataVersion}`;
}

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  if (bytes >= 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  return `${bytes} B`;
}
