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
  listInstances,
  listSchematics,
  readSchematic,
} from "../ipc";
import type {
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
 * 受管理目录里的 `.litematic` 文件在这里列表、预览与管理：
 * - 导入：前端读文件内容传给后端（IPC 只传名字和字节，不传路径），单文件
 *   上限 16 MiB；更大的文件可以直接放进受管理目录后刷新；
 * - 预览：后端完整解码后交给 three.js 做 3D 渲染，可旋转缩放；
 * - 复制到实例：把文件复制进实例的 `schematics/` 目录，跨版本使用时
 *   页面会提示方块状态可能随数据版本变化。
 */
export function SchematicsPage({ page }: SchematicsPageProps) {
  const desktop = isTauri();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [detail, setDetail] = useState<SchematicDetail | null>(null);
  const [busy, setBusy] = useState(false);
  const [targetInstanceId, setTargetInstanceId] = useState<string | null>(null);

  const loadList = useCallback(async () => {
    const result = await listSchematics();
    return result;
  }, []);

  const loadInstances = useCallback(async () => {
    const result = await listInstances();
    return result;
  }, []);

  const { status, data, error, reload } = useQuery(loadList, desktop);
  const instancesQuery = useQuery(loadInstances, desktop);
  const listing = data ?? emptySchematicListing;
  const instances = instancesQuery.data?.instances ?? [];

  const handleImportClick = useCallback(() => {
    setImportError(null);
    fileInputRef.current?.click();
  }, []);

  const handleFileChosen = useCallback(
    async (event: React.ChangeEvent<HTMLInputElement>) => {
      const file = event.target.files?.[0];
      event.target.value = "";
      if (!file) {
        return;
      }
      setImportError(null);
      if (!file.name.endsWith(".litematic")) {
        setImportError("只能导入 .litematic 文件");
        return;
      }
      if (file.size > IMPORT_LIMIT_BYTES) {
        setImportError(
          `文件超过 16 MiB 导入上限。请把它直接放进受管理的投影图目录后刷新。`,
        );
        return;
      }
      setBusy(true);
      try {
        const bytes = new Uint8Array(await file.arrayBuffer());
        await importSchematic(file.name, bytes);
        reload();
      } catch (caught) {
        setImportError(describeError(caught));
      } finally {
        setBusy(false);
      }
    },
    [reload],
  );

  const handlePreview = useCallback(async (item: SchematicSummary) => {
    setActionError(null);
    setBusy(true);
    try {
      const loaded = await readSchematic(item.fileName);
      setDetail(loaded);
    } catch (caught) {
      setDetail(null);
      setActionError(describeError(caught));
    } finally {
      setBusy(false);
    }
  }, []);

  const handleDelete = useCallback(
    async (item: SchematicSummary) => {
      if (!window.confirm(`确定删除「${item.fileName}」吗？此操作无法撤销。`)) {
        return;
      }
      setActionError(null);
      setBusy(true);
      try {
        await deleteSchematic(item.fileName);
        // 函数式更新：被删的就是当前预览时才收起，不把 detail 拉进依赖。
        setDetail((previous) =>
          previous?.fileName === item.fileName ? null : previous,
        );
        reload();
      } catch (caught) {
        setActionError(describeError(caught));
      } finally {
        setBusy(false);
      }
    },
    [reload],
  );

  const handleCopy = useCallback(
    async (item: SchematicSummary) => {
      if (!targetInstanceId) {
        setActionError("先选择要复制到的实例。");
        return;
      }
      setActionError(null);
      setBusy(true);
      try {
        await copySchematicToInstance(item.fileName, targetInstanceId);
        setActionError(null);
        window.alert("已复制到实例的 schematics 目录。");
      } catch (caught) {
        setActionError(describeError(caught));
      } finally {
        setBusy(false);
      }
    },
    [targetInstanceId],
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
            disabled={!desktop || busy}
            onClick={handleImportClick}
          >
            导入投影图
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
        <DesktopOnlyNotice body="浏览器预览读不到本机投影图。用桌面版可以导入、预览和管理 .litematic 文件。" />
      ) : null}

      {importError ? (
        <MessageBar intent="warning">
          <MessageBarBody>{importError}</MessageBarBody>
        </MessageBar>
      ) : null}
      {actionError ? (
        <MessageBar intent="error">
          <MessageBarBody>{actionError}</MessageBarBody>
        </MessageBar>
      ) : null}

      <section className="status-panel list-panel" aria-labelledby="schematics-heading">
        <div className="list-toolbar">
          <div className="toolbar-heading">
            <Text id="schematics-heading" as="h2" size={400} weight="semibold">
              受管理的投影图
            </Text>
            {desktop ? (
              <Badge appearance="tint">{listing.schematics.length} 个文件</Badge>
            ) : null}
          </div>
          {desktop && instances.length > 0 ? (
            <div className="toolbar-actions">
              <Text className="muted">复制到实例</Text>
              <Dropdown
                placeholder="选择实例"
                value={
                  instances.find((instance) => instance.id === targetInstanceId)
                    ?.name ?? undefined
                }
                onOptionSelect={(_, option) =>
                  setTargetInstanceId(option.optionValue ?? null)
                }
              >
                {instances.map((instance) => (
                  <Option key={instance.id} value={instance.id} text={instance.name}>
                    {instance.name}（{instance.version}）
                  </Option>
                ))}
              </Dropdown>
            </div>
          ) : null}
        </div>

        {!desktop ? null : status === "loading" ? (
          <ListSkeleton />
        ) : status === "error" ? (
          <ErrorState
            title="投影图列表读取失败"
            message={error ?? "未知错误"}
            onRetry={reload}
          />
        ) : listing.schematics.length === 0 && listing.failed.length === 0 ? (
          <EmptyState
            icon={<Cube24Regular />}
            title="还没有投影图"
            body="点右上角导入 .litematic 文件，或把文件直接放进受管理的投影图目录。"
          />
        ) : (
          <ul className="schematic-list">
            {listing.schematics.map((item) => (
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
                    onClick={() => void handlePreview(item)}
                  >
                    预览
                  </Button>
                  <Button
                    size="small"
                    disabled={busy || !targetInstanceId}
                    onClick={() => void handleCopy(item)}
                  >
                    复制到实例
                  </Button>
                  <Button
                    size="small"
                    icon={<Delete24Regular />}
                    disabled={busy}
                    onClick={() => void handleDelete(item)}
                  >
                    删除
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}

        {listing.failed.length > 0 ? (
          <MessageBar intent="warning">
            <MessageBarBody>
              <Warning24Regular /> {listing.failed.length} 个文件无法解析：
              {listing.failed.map((failed) => failed.fileName).join("、")}
            </MessageBarBody>
          </MessageBar>
        ) : null}
      </section>

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
