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
  ArrowUpload24Regular,
  Cube24Regular,
  Delete24Regular,
  Eye24Regular,
  Warning24Regular,
} from "@fluentui/react-icons";
import { useCallback, useRef, useState } from "react";
import { SchematicPreview } from "../components/SchematicPreview";
import { EmptyState } from "../components/EmptyState";
import {
  DesktopOnlyNotice,
  ErrorState,
  ListSkeleton,
} from "../components/PageState";
import { PageHeader } from "../components/PageHeader";
import {
  IMPORT_LIMIT_BYTES,
  copySchematicToInstance,
  deleteSchematic,
  emptySchematicListing,
  importSchematic,
  isTauri,
  listSchematics,
  readSchematic,
} from "../ipc";
import type {
  InstanceSchematics,
  SchematicDetail,
  SchematicSummary,
} from "../ipc";
import { useQuery } from "../lib/query";
import type { PageDefinition } from "../navigation";

type SchematicsPageProps = {
  page: PageDefinition;
};

/**
 * 投影图页。
 *
 * 后端自动扫描每个实例游戏目录下的 `schematics/`，这里按实例分组展示：
 * - 导入：选一个实例后从文件读内容传给后端（IPC 只传实例 ID、名字和字节，
 *   不传路径），单文件上限 16 MiB；
 * - 预览：后端完整解码后交给 three.js 做 3D 渲染，可旋转缩放；
 * - 跨版本：页顶选一个「复制目标实例」，把投影复制过去；数据版本不同时
 *   预览面板会提示方块状态可能随版本变化。
 */
export function SchematicsPage({ page }: SchematicsPageProps) {
  const desktop = isTauri();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [importTargetId, setImportTargetId] = useState<string | null>(null);
  const [copyTargetId, setCopyTargetId] = useState<string | null>(null);
  const [detail, setDetail] = useState<SchematicDetail | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const loadList = useCallback(async () => listSchematics(), []);
  const { status, data, error: listError, reload } = useQuery(loadList, desktop);
  const listing = data ?? emptySchematicListing;

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
        setError("文件超过 16 MiB 导入上限。请把它直接放进该实例的 schematics 目录后刷新。");
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

  const handlePreview = useCallback(
    async (instanceId: string, item: SchematicSummary) => {
      setError(null);
      setBusy(true);
      try {
        const loaded = await readSchematic(instanceId, item.fileName);
        setDetail(loaded);
      } catch (caught) {
        setDetail(null);
        setError(describeError(caught));
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  const handleDelete = useCallback(
    async (instanceId: string, item: SchematicSummary) => {
      if (!window.confirm(`确定删除「${item.fileName}」吗？此操作无法撤销。`)) {
        return;
      }
      setError(null);
      setNotice(null);
      setBusy(true);
      try {
        await deleteSchematic(instanceId, item.fileName);
        // 函数式更新：被删的就是当前预览时才收起。
        setDetail((previous) =>
          previous?.instanceId === instanceId && previous?.fileName === item.fileName
            ? null
            : previous,
        );
        setNotice(`已删除 ${item.fileName}`);
        reload();
      } catch (caught) {
        setError(describeError(caught));
      } finally {
        setBusy(false);
      }
    },
    [reload],
  );

  const handleCopy = useCallback(
    async (fromInstanceId: string, item: SchematicSummary) => {
      if (!copyTargetId) {
        setError("先在页顶选择要复制到的目标实例。");
        return;
      }
      if (copyTargetId === fromInstanceId) {
        setError("源实例和目标实例是同一个。");
        return;
      }
      setError(null);
      setNotice(null);
      setBusy(true);
      try {
        await copySchematicToInstance(fromInstanceId, copyTargetId, item.fileName);
        const target = listing.instances.find((instance) => instance.id === copyTargetId);
        setNotice(`已复制到「${target?.name ?? copyTargetId}」`);
        reload();
      } catch (caught) {
        setError(describeError(caught));
      } finally {
        setBusy(false);
      }
    },
    [copyTargetId, listing.instances, reload],
  );

  return (
    <>
      <PageHeader
        title={page.label}
        description={page.summary}
        action={
          <Button
            appearance="primary"
            icon={<ArrowUpload24Regular />}
            disabled={!desktop || busy || listing.instances.length === 0}
            onClick={() => {
              if (listing.instances.length > 0) {
                handleImportClick(listing.instances[0].id);
              }
            }}
          >
            导入到第一个实例
          </Button>
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
        <DesktopOnlyNotice body="浏览器预览读不到本机实例。用桌面版可以自动识别各实例里的投影图。" />
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
          title="还没有实例"
          body="投影图跟着实例走：先到「实例」页创建或导入实例，再把 .litematic 放进它的 schematics 目录，或在下面导入。"
        />
      ) : (
        <>
          {listing.instances.length > 1 ? (
            <section className="status-panel" aria-label="跨版本复制目标">
              <div className="toolbar-actions">
                <Text className="muted">复制目标实例</Text>
                <Dropdown
                  placeholder="选择目标实例（复制用）"
                  value={
                    listing.instances.find((instance) => instance.id === copyTargetId)
                      ?.name ?? undefined
                  }
                  onOptionSelect={(_, option) =>
                    setCopyTargetId(option.optionValue ?? null)
                  }
                >
                  {listing.instances.map((instance) => (
                    <Option
                      key={instance.id}
                      value={instance.id}
                      text={`${instance.name}（${instance.version}）`}
                    >
                      {instance.name}（{instance.version}）
                    </Option>
                  ))}
                </Dropdown>
              </div>
            </section>
          ) : null}

          {listing.instances.map((instance) => (
            <InstanceGroup
              key={instance.id}
              instance={instance}
              busy={busy}
              copyReady={copyTargetId !== null}
              onImport={handleImportClick}
              onPreview={handlePreview}
              onCopy={handleCopy}
              onDelete={handleDelete}
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
            不同的实例时，部分方块状态可能随版本变化，进游戏后请核对。
          </p>
        </section>
      ) : null}
    </>
  );
}

type InstanceGroupProps = {
  instance: InstanceSchematics;
  busy: boolean;
  copyReady: boolean;
  onImport: (instanceId: string) => void;
  onPreview: (instanceId: string, item: SchematicSummary) => void;
  onCopy: (instanceId: string, item: SchematicSummary) => void;
  onDelete: (instanceId: string, item: SchematicSummary) => void;
};

function InstanceGroup({
  instance,
  busy,
  copyReady,
  onImport,
  onPreview,
  onCopy,
  onDelete,
}: InstanceGroupProps) {
  return (
    <section className="status-panel list-panel" aria-label={`实例 ${instance.name} 的投影图`}>
      <div className="list-toolbar">
        <div className="toolbar-heading">
          <Text as="h2" size={400} weight="semibold">
            {instance.name}
          </Text>
          <Badge appearance="tint">{instance.version}</Badge>
          <Badge appearance="tint">{instance.schematics.length} 个投影</Badge>
        </div>
        <div className="toolbar-actions">
          <Button
            size="small"
            icon={<ArrowUpload24Regular />}
            disabled={busy}
            onClick={() => onImport(instance.id)}
          >
            导入到该实例
          </Button>
        </div>
      </div>

      {instance.schematics.length === 0 && instance.failed.length === 0 ? (
        <Text className="muted" size={300}>
          这个实例的 schematics 目录里还没有投影文件。
        </Text>
      ) : (
        <ul className="schematic-list">
          {instance.schematics.map((item) => (
            <li key={item.fileName} className="schematic-row">
              <div className="schematic-info">
                <Text weight="semibold">{item.metadata.name || item.fileName}</Text>
                <Text className="muted" size={300}>
                  {item.fileName} · {formatBytes(item.sizeBytes)} ·{" "}
                  {item.metadata.totalVolume.toLocaleString("zh-CN")} 格 ·{" "}
                  {item.metadata.regionCount} 个区域
                </Text>
                <Text className="muted" size={300}>
                  {describeOrigin(item)}
                </Text>
              </div>
              <div className="schematic-actions">
                <Button
                  size="small"
                  icon={<Eye24Regular />}
                  disabled={busy}
                  onClick={() => onPreview(instance.id, item)}
                >
                  预览
                </Button>
                <Button
                  size="small"
                  disabled={busy || !copyReady}
                  onClick={() => onCopy(instance.id, item)}
                >
                  复制到目标实例
                </Button>
                <Button
                  size="small"
                  icon={<Delete24Regular />}
                  disabled={busy}
                  onClick={() => onDelete(instance.id, item)}
                >
                  删除
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}

      {instance.failed.length > 0 ? (
        <MessageBar intent="warning">
          <MessageBarBody>
            <Warning24Regular /> {instance.failed.length} 个文件无法解析：
            {instance.failed.map((failed) => failed.fileName).join("、")}
          </MessageBarBody>
        </MessageBar>
      ) : null}
    </section>
  );
}

function describeOrigin(item: SchematicSummary): string {
  const release = item.metadata.createdForRelease;
  return release
    ? `出自 MC ${release}（数据版本 ${item.metadata.minecraftDataVersion}）`
    : `数据版本 ${item.metadata.minecraftDataVersion}（早于记录表）`;
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

function describeError(error: unknown): string {
  if (typeof error === "string") {
    return error;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}
