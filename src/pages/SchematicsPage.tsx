import {
  Badge,
  Button,
  Dropdown,
  MessageBar,
  MessageBarBody,
  Option,
  Text,
} from "@fluentui/react-components";
import {
  ArrowSync24Regular,
  ArrowUpload24Regular,
  ClipboardPaste24Regular,
  Cube24Regular,
  FolderOpen24Regular,
  Settings24Regular,
  Warning24Regular,
} from "@fluentui/react-icons";
import { useCallback, useMemo, useRef, useState } from "react";
import { EmptyState } from "../components/EmptyState";
import { PageHeader } from "../components/PageHeader";
import {
  DesktopOnlyNotice,
  ErrorState,
  ListSkeleton,
} from "../components/PageState";
import { SchematicPreview } from "../components/SchematicPreview";
import { SchematicTree } from "../components/SchematicTree";
import {
  IMPORT_LIMIT_BYTES,
  deleteSchematic,
  emptySchematicListing,
  importSchematic,
  importSchematicFolder,
  isTauri,
  listInstances,
  listSchematics,
  pasteSchematic,
  readSchematic,
  setGameDirectoryForAllInstances,
} from "../ipc";
import type {
  InstanceSchematics,
  InstanceSummary,
  SchematicDetail,
  SchematicSummary,
} from "../ipc";
import { useQuery } from "../lib/query";
import { buildSchematicSources } from "../lib/schematicTree";
import type { SchematicSourceTree } from "../lib/schematicTree";
import type { PageDefinition } from "../navigation";

type SchematicsPageProps = {
  page: PageDefinition;
};

/** 多选状态：一次只在一个版本里选，避免跨版本批量操作把语义搞混。 */
type Selection = { instanceId: string; paths: Set<string> };

/** 剪贴板：复制或剪切了哪些投影，粘贴时按 `move` 决定要不要删源。 */
type Clipboard = { instanceId: string; paths: string[]; move: boolean };

const EMPTY_SET: ReadonlySet<string> = new Set();

/**
 * 投影图页。
 *
 * 后端递归扫描每个实例游戏目录下的 `schematics/`，**只返回有投影的版本**，
 * 并用 `relativePath` 带回子文件夹结构。这里按版本分组做成手风琴：点标题
 * 展开，展开后是文件夹树。
 *
 * 右键菜单：文件行可以复制 / 剪切 / 删除 / 多选，文件夹行可以「粘贴到此处」。
 * 复制和剪切都只记在剪贴板里，粘贴时才真正落盘——同版本内整理目录和跨版本
 * 共享是同一套操作。预览仍由 three.js 渲染。
 */
export function SchematicsPage({ page }: SchematicsPageProps) {
  const desktop = isTauri();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [importTargetId, setImportTargetId] = useState<string | null>(null);
  const [detail, setDetail] = useState<SchematicDetail | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // 版本默认只展开第一个；用户点过之后以点击结果为准。
  const [expandedVersions, setExpandedVersions] = useState<Record<string, boolean>>({});
  // 文件夹默认展开，这里只记被折叠的。
  // 文件夹默认**折叠**：一个版本的 schematics 根上能有七八百个文件，
  // 全部展开就是一次渲染上千行，页面直接卡住。只记用户主动展开的。
  const [expandedFolders, setExpandedFolders] = useState<Set<string>>(new Set());
  const [selection, setSelection] = useState<Selection | null>(null);
  const [clipboard, setClipboard] = useState<Clipboard | null>(null);

  const loadList = useCallback(async () => listSchematics(), []);
  const {
    status,
    data,
    error: listError,
    reload,
  } = useQuery(loadList, desktop);
  const listing = data ?? emptySchematicListing;

  // 导入目标用完整实例列表：还没有投影的版本不会出现在上面的列表里。
  const loadInstances = useCallback(async () => listInstances(), []);
  const instancesQuery = useQuery(loadInstances, desktop);
  const allInstances = instancesQuery.data?.instances ?? [];

  const sources = useMemo(
    () =>
      new Map(
        listing.instances.map((instance) => [
          instance.id,
          buildSchematicSources(instance.schematics),
        ]),
      ),
    [listing.instances],
  );

  /** 老实例还没有游戏目录时，让用户选一次 `.minecraft`，后端写回并备份。 */
  const handleSetGameDirectory = useCallback(async () => {
    setError(null);
    setNotice(null);
    setBusy(true);
    try {
      const report = await setGameDirectoryForAllInstances();
      if (report.cancelled) {
        return;
      }
      const parts = [`已为 ${report.updated} 个实例设置游戏目录`];
      if (report.skipped > 0) {
        parts.push(`${report.skipped} 个已有设置`);
      }
      if (report.failed.length > 0) {
        parts.push(`${report.failed.length} 个失败`);
      }
      setNotice(parts.join("，"));
      reload();
      instancesQuery.reload();
    } catch (caught) {
      setError(describeError(caught));
    } finally {
      setBusy(false);
    }
  }, [instancesQuery, reload]);

  const expandedByInstance = useMemo(() => {
    const map = new Map<string, Set<string>>();
    for (const key of expandedFolders) {
      const index = key.indexOf("|");
      const id = key.slice(0, index);
      const path = key.slice(index + 1);
      const set = map.get(id) ?? new Set<string>();
      set.add(path);
      map.set(id, set);
    }
    return map;
  }, [expandedFolders]);

  const targetId =
    importTargetId ?? listing.instances[0]?.id ?? allInstances[0]?.id ?? null;

  const handleImportClick = useCallback((instanceId: string) => {
    setError(null);
    setNotice(null);
    setImportTargetId(instanceId);
    fileInputRef.current?.click();
  }, []);

  const handleFileChosen = useCallback(
    async (event: React.ChangeEvent<HTMLInputElement>) => {
      const file = event.target.files?.[0];
      event.target.value = "";
      if (!file || !importTargetId) {
        return;
      }
      setError(null);
      if (!file.name.endsWith(".litematic")) {
        setError("只能导入 .litematic 文件");
        return;
      }
      if (file.size > IMPORT_LIMIT_BYTES) {
        setError("文件超过 16 MiB 导入上限。请把它直接放进该版本的 schematics 目录后刷新。");
        return;
      }
      setBusy(true);
      try {
        const bytes = new Uint8Array(await file.arrayBuffer());
        await importSchematic(importTargetId, file.name, bytes);
        setNotice(`已导入 ${file.name}`);
        reload();
      } catch (caught) {
        setError(describeError(caught));
      } finally {
        setBusy(false);
      }
    },
    [importTargetId, reload],
  );

  /** 整个文件夹导入：后端打开选择框，递归复制并保留子文件夹结构。 */
  const handleImportFolder = useCallback(async () => {
    if (!targetId) {
      return;
    }
    setError(null);
    setNotice(null);
    setBusy(true);
    try {
      const result = await importSchematicFolder(targetId);
      if (!result) {
        return;
      }
      const skipped =
        result.skipped.length > 0
          ? `，跳过 ${result.skipped.length} 个（${result.skipped
              .slice(0, 3)
              .map((item) => item.relativePath)
              .join("、")}${result.skipped.length > 3 ? " 等" : ""}）`
          : "";
      setNotice(`已导入 ${result.imported} 个投影${skipped}`);
      reload();
    } catch (caught) {
      setError(describeError(caught));
    } finally {
      setBusy(false);
    }
  }, [reload, targetId]);

  const handlePreview = useCallback(
    async (instanceId: string, item: SchematicSummary) => {
      setError(null);
      setBusy(true);
      try {
        setDetail(await readSchematic(instanceId, item.relativePath));
      } catch (caught) {
        setDetail(null);
        setError(describeError(caught));
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  const handleCopy = useCallback(
    (instanceId: string, paths: string[], move: boolean) => {
      setError(null);
      setClipboard({ instanceId, paths, move });
      setNotice(
        `已${move ? "剪切" : "复制"} ${paths.length} 个投影。右键目标文件夹选「粘贴到此处」，或点版本标题上的「粘贴到根目录」。`,
      );
    },
    [],
  );

  const handlePaste = useCallback(
    async (instanceId: string, folder: string) => {
      if (!clipboard) {
        return;
      }
      setError(null);
      setNotice(null);
      setBusy(true);
      const failures: string[] = [];
      let done = 0;
      for (const path of clipboard.paths) {
        try {
          await pasteSchematic(
            clipboard.instanceId,
            path,
            instanceId,
            folder,
            clipboard.move,
          );
          done += 1;
        } catch (caught) {
          failures.push(describeError(caught));
        }
      }
      setBusy(false);
      if (failures.length > 0) {
        setError(`${failures.length} 个投影粘贴失败：${failures[0]}`);
      }
      if (done > 0) {
        setNotice(
          `已粘贴 ${done} 个投影到${folder ? `「${folder}」` : "根目录"}`,
        );
        if (clipboard.move) {
          setClipboard(null);
          setSelection(null);
        }
        reload();
      }
    },
    [clipboard, reload],
  );

  const handleDelete = useCallback(
    async (instanceId: string, paths: string[]) => {
      const label = paths.length === 1 ? `「${paths[0]}」` : `这 ${paths.length} 个投影`;
      if (!window.confirm(`确定删除${label}吗？此操作无法撤销。`)) {
        return;
      }
      setError(null);
      setNotice(null);
      setBusy(true);
      const failures: string[] = [];
      let done = 0;
      for (const path of paths) {
        try {
          await deleteSchematic(instanceId, path);
          done += 1;
        } catch (caught) {
          failures.push(describeError(caught));
        }
      }
      setBusy(false);
      if (failures.length > 0) {
        setError(`${failures.length} 个投影删除失败：${failures[0]}`);
      }
      if (done > 0) {
        setNotice(`已删除 ${done} 个投影`);
        setDetail((previous) =>
          previous &&
          previous.instanceId === instanceId &&
          paths.includes(previous.relativePath)
            ? null
            : previous,
        );
        setSelection(null);
        reload();
      }
    },
    [reload],
  );

  const toggleFolder = useCallback((instanceId: string, path: string) => {
    const key = `${instanceId}|${path}`;
    setExpandedFolders((previous) => {
      const next = new Set(previous);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }, []);

  const toggleSelect = useCallback((instanceId: string, path: string) => {
    setSelection((previous) => {
      if (!previous || previous.instanceId !== instanceId) {
        return { instanceId, paths: new Set([path]) };
      }
      const paths = new Set(previous.paths);
      if (paths.has(path)) {
        paths.delete(path);
      } else {
        paths.add(path);
      }
      return paths.size === 0 ? null : { instanceId, paths };
    });
  }, []);

  const startSelection = useCallback((instanceId: string, path: string) => {
    setSelection({ instanceId, paths: new Set([path]) });
  }, []);

  const selectAll = useCallback((instanceId: string, paths: string[]) => {
    setSelection(paths.length === 0 ? null : { instanceId, paths: new Set(paths) });
  }, []);

  const selectedPaths = selection ? [...selection.paths] : [];

  return (
    <>
      <PageHeader
        title={page.label}
        description={page.summary}
        action={
          <div className="toolbar-actions">
            <Dropdown
              aria-label="导入目标版本"
              placeholder="导入目标版本"
              disabled={!desktop || allInstances.length === 0}
              value={describeInstance(
                allInstances.find((instance) => instance.id === targetId),
              )}
              onOptionSelect={(_, option) =>
                setImportTargetId(option.optionValue ?? null)
              }
            >
              {allInstances.map((instance) => (
                <Option
                  key={instance.id}
                  value={instance.id}
                  text={describeInstance(instance)}
                >
                  {describeInstance(instance)}
                </Option>
              ))}
            </Dropdown>
            <Button
              appearance="primary"
              icon={<ArrowUpload24Regular />}
              disabled={!desktop || busy || targetId === null}
              onClick={() => {
                if (targetId) {
                  handleImportClick(targetId);
                }
              }}
            >
              导入投影
            </Button>
            <Button
              icon={<FolderOpen24Regular />}
              disabled={!desktop || busy || targetId === null}
              onClick={() => void handleImportFolder()}
            >
              导入文件夹
            </Button>
            <Button
              icon={<ArrowSync24Regular />}
              disabled={!desktop || busy || status === "loading"}
              onClick={() => {
                setNotice(null);
                setError(null);
                reload();
                // 实例列表也一起刷：刚在别的页面新建或导入的版本要能选到。
                instancesQuery.reload();
              }}
            >
              刷新
            </Button>
            {listing.missingGameDirectory > 0 ? (
              <Button
                icon={<Settings24Regular />}
                disabled={!desktop || busy}
                onClick={() => void handleSetGameDirectory()}
              >
                设置游戏目录（{listing.missingGameDirectory}）
              </Button>
            ) : null}
          </div>
        }
      />
      <input
        ref={fileInputRef}
        type="file"
        accept=".litematic"
        hidden
        onChange={handleFileChosen}
      />

      {!desktop ? (
        <DesktopOnlyNotice body="浏览器预览读不到本机实例。用桌面版可以自动识别各版本里的投影图。" />
      ) : null}

      {error ? (
        <MessageBar intent="error">
          <MessageBarBody>{error}</MessageBarBody>
        </MessageBar>
      ) : null}
      {notice ? (
        <MessageBar intent="success">
          <MessageBarBody>{notice}</MessageBarBody>
        </MessageBar>
      ) : null}

      {clipboard ? (
        <section className="status-panel schematic-clipboard" aria-label="剪贴板">
          <Text>
            已{clipboard.move ? "剪切" : "复制"} {clipboard.paths.length} 个投影
            {clipboard.paths.length === 1 ? `：${clipboard.paths[0]}` : ""}
          </Text>
          <Button size="small" disabled={busy} onClick={() => setClipboard(null)}>
            取消
          </Button>
        </section>
      ) : null}

      {selection ? (
        <section className="status-panel schematic-clipboard" aria-label="多选操作">
          <Text>已选 {selection.paths.size} 项</Text>
          <div className="toolbar-actions">
            <Button
              size="small"
              disabled={busy}
              onClick={() => handleCopy(selection.instanceId, selectedPaths, false)}
            >
              复制
            </Button>
            <Button
              size="small"
              disabled={busy}
              onClick={() => handleCopy(selection.instanceId, selectedPaths, true)}
            >
              剪切
            </Button>
            <Button
              size="small"
              disabled={busy}
              onClick={() => handleDelete(selection.instanceId, selectedPaths)}
            >
              删除
            </Button>
            <Button size="small" disabled={busy} onClick={() => setSelection(null)}>
              取消选择
            </Button>
          </div>
        </section>
      ) : null}

      {!desktop ? null : status === "loading" ? (
        <ListSkeleton />
      ) : status === "error" ? (
        <ErrorState
          title="投影图扫描失败"
          message={listError ?? "未知错误"}
          onRetry={reload}
        />
      ) : listing.instances.length === 0 ? (
        <EmptyState
          icon={<Cube24Regular />}
          title="还没有版本带投影文件"
          body="投影图跟着版本走：把 .litematic 放进某个版本游戏目录的 schematics 文件夹（可以带子文件夹），或用上面的「导入」按钮。没有 schematics 文件夹的版本不会出现在这里。"
        />
      ) : (
        <>
          {listing.instances.map((instance, index) => (
            <VersionPanel
              key={instance.id}
              instance={instance}
              sources={sources.get(instance.id) ?? []}
              open={expandedVersions[instance.id] ?? index === 0}
              busy={busy}
              clipboardReady={clipboard !== null}
              selectionMode={selection?.instanceId === instance.id}
              selected={
                selection?.instanceId === instance.id ? selection.paths : EMPTY_SET
              }
              expandedFolders={
                expandedByInstance.get(instance.id) ?? EMPTY_SET
              }
              onToggleVersion={() =>
                setExpandedVersions((previous) => ({
                  ...previous,
                  [instance.id]: !(previous[instance.id] ?? index === 0),
                }))
              }
              onImport={() => handleImportClick(instance.id)}
              onPaste={(folder) => handlePaste(instance.id, folder)}
              onToggleFolder={(path) => toggleFolder(instance.id, path)}
              onToggleSelect={(path) => toggleSelect(instance.id, path)}
              onStartSelection={(path) => startSelection(instance.id, path)}
              onSelectAll={(paths) => selectAll(instance.id, paths)}
              onPreview={(item) => handlePreview(instance.id, item)}
              onCopy={(paths, move) => handleCopy(instance.id, paths, move)}
              onDelete={(paths) => handleDelete(instance.id, paths)}
            />
          ))}

          {listing.skipped.length > 0 ? (
            <Text className="muted" size={300}>
              另有 {listing.skipped.length} 个无法读取的实例记录被跳过。
            </Text>
          ) : null}
        </>
      )}

      {detail ? (
        <section className="status-panel preview-panel" aria-label="3D 预览">
          <div className="preview-heading">
            <Text as="h2" size={400} weight="semibold">
              3D 预览：{detail.metadata.name || detail.fileName}
            </Text>
            <Text className="muted" size={300}>
              拖动旋转 · 滚轮缩放 · 右键平移
            </Text>
          </div>
          <SchematicPreview detail={detail} />
          <p className="muted preview-note">
            此投影出自 MC {detail.metadata.createdForRelease ?? "未知版本"}
            （数据版本 {detail.metadata.minecraftDataVersion}）。复制到数据版本
            不同的版本时，部分方块状态可能随版本变化，进游戏后请核对。
          </p>
        </section>
      ) : null}
    </>
  );
}

type VersionPanelProps = {
  instance: InstanceSchematics;
  sources: SchematicSourceTree[];
  open: boolean;
  busy: boolean;
  clipboardReady: boolean;
  selectionMode: boolean;
  selected: ReadonlySet<string>;
  expandedFolders: ReadonlySet<string>;
  onToggleVersion: () => void;
  onImport: () => void;
  onPaste: (folder: string) => void;
  onToggleFolder: (path: string) => void;
  onToggleSelect: (path: string) => void;
  onStartSelection: (path: string) => void;
  onSelectAll: (paths: string[]) => void;
  onPreview: (item: SchematicSummary) => void;
  onCopy: (paths: string[], move: boolean) => void;
  onDelete: (paths: string[]) => void;
};

function VersionPanel({
  instance,
  sources,
  open,
  busy,
  clipboardReady,
  selectionMode,
  selected,
  expandedFolders,
  onToggleVersion,
  onImport,
  onPaste,
  onToggleFolder,
  onToggleSelect,
  onStartSelection,
  onSelectAll,
  onPreview,
  onCopy,
  onDelete,
}: VersionPanelProps) {
  return (
    <section className="status-panel schematic-version" aria-label={`版本 ${instance.name}`}>
      <div className="schematic-version-header">
        <button
          type="button"
          className="schematic-version-toggle"
          aria-expanded={open}
          aria-label={`${instance.name}（${instance.version}）`}
          onClick={onToggleVersion}
        >
          <span className="schematic-version-name">{instance.name}</span>
          <Badge appearance="tint">{instance.version}</Badge>
          <Badge appearance="tint">{instance.schematics.length} 个投影</Badge>
        </button>
        <div className="toolbar-actions">
          <Button
            size="small"
            icon={<ClipboardPaste24Regular />}
            disabled={busy || !clipboardReady}
            onClick={() => onPaste("")}
          >
            粘贴到根目录
          </Button>
          <Button
            size="small"
            icon={<ArrowUpload24Regular />}
            disabled={busy}
            onClick={onImport}
          >
            导入
          </Button>
        </div>
      </div>

      {open ? (
        <div className="schematic-version-body">
          {instance.schematics.length > 0 && sources.length > 0 ? (
            <>
              {sources.map((source) => (
                <div className="schematic-source" key={`${source.origin}|${source.originVersion}`}>
                  <div className="schematic-source-header">
                    <Text size={200} className="muted">
                      来源：<code>{source.label}</code>
                    </Text>
                    <Badge appearance="tint">{source.total} 个</Badge>
                  </div>
                  <SchematicTree
                    root={source.root}
                    busy={busy}
                    selectionMode={selectionMode}
                    selected={selected}
                    expandedFolders={expandedFolders}
                    clipboardReady={clipboardReady}
                    onToggleFolder={onToggleFolder}
                    onToggleSelect={onToggleSelect}
                    onPreview={onPreview}
                    onCopy={onCopy}
                    onPaste={onPaste}
                    onDelete={onDelete}
                    onStartSelection={onStartSelection}
                    onSelectAll={onSelectAll}
                  />
                </div>
              ))}
            </>
          ) : (
            <Text className="muted" size={300}>
              这个版本的 schematics 目录里只有无法解析的文件。
            </Text>
          )}

          {instance.failed.length > 0 ? (
            <MessageBar intent="warning">
              <MessageBarBody>
                <Warning24Regular /> {instance.failed.length} 个文件无法解析：
                {instance.failed.map((failed) => failed.relativePath).join("、")}
              </MessageBarBody>
            </MessageBar>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}

function describeError(error: unknown): string {
  if (typeof error === "string") {
    return error;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

/** 下拉框里统一用「名字（版本）」；`value` 必须和 `Option` 的文本一致。 */
function describeInstance(instance: InstanceSummary | undefined): string {
  return instance ? `${instance.name}（${instance.version}）` : "";
}
