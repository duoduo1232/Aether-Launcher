import {
  Badge,
  Button,
  MessageBar,
  MessageBarBody,
  SearchBox,
  Text,
} from "@fluentui/react-components";
import {
  Add24Regular,
  ArrowSync24Regular,
  BoxMultiple24Regular,
  ChevronRight24Regular,
  Warning24Regular,
} from "@fluentui/react-icons";
import { useCallback, useMemo, useState } from "react";
import { CreateInstanceDialog } from "../components/CreateInstanceDialog";
import { ImportInstancesDialog } from "../components/ImportInstancesDialog";
import { EmptyState } from "../components/EmptyState";
import { InstanceLaunchPanel } from "../components/InstanceLaunchPanel";
import { InstanceList } from "../components/InstanceList";
import {
  DesktopOnlyNotice,
  ErrorState,
  ListSkeleton,
} from "../components/PageState";
import { PageHeader } from "../components/PageHeader";
import {
  deleteInstance,
  emptyInstanceListing,
  isTauri,
  listInstances,
} from "../ipc";
import type {
  InstanceCategory,
  InstanceListing,
  InstanceSummary,
} from "../ipc";
import {
  describeSkippedReason,
  filterInstances,
  sortInstances,
  summariseInstances,
} from "../lib/instances";
import { formatMemoryMiB } from "../lib/memory";
import { useQuery } from "../lib/query";
import { loadJavaRuntimeKey, loadMemoryMiB } from "../lib/settings";
import type { PageDefinition } from "../navigation";

/** 分组顺序：原版在最前，整合包在最后。 */
const CATEGORY_ORDER: InstanceCategory[] = [
  "vanilla",
  "forge",
  "neoforge",
  "fabric",
  "quilt",
  "liteloader",
  "modpack",
];

const CATEGORY_LABEL: Record<InstanceCategory, string> = {
  vanilla: "原版游戏",
  forge: "Forge",
  neoforge: "NeoForge",
  fabric: "Fabric",
  quilt: "Quilt",
  liteloader: "LiteLoader",
  modpack: "整合包",
};

type InstancesPageProps = {
  page: PageDefinition;
  /**
   * 打开时直接弹出导入对话框。首页的「导入实例」按钮跳过来时用它，
   * 免得用户再点一次。
   */
  openImport?: boolean;
};

/**
 * 实例页。
 *
 * 数据来自 `list_instances`；浏览器预览里没有 Tauri，返回空列表并显示
 * 「需要桌面版」的提示，不把环境差异当成错误。
 */
export function InstancesPage({ page, openImport = false }: InstancesPageProps) {
  const desktop = isTauri();
  const [query, setQuery] = useState("");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(openImport);
  const [listing, setListing] = useState<InstanceListing>(emptyInstanceListing);
  const [notice, setNotice] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [memoryMiB] = useState(loadMemoryMiB);
  const [javaKey] = useState(loadJavaRuntimeKey);

  const load = useCallback(async () => {
    const result = await listInstances();
    setListing(result);
    return result;
  }, []);

  const { status, error, reload } = useQuery(load, desktop);

  // 浏览器预览下 useQuery 不运行，直接用空列表展示
  const instances = useMemo(
    () => (desktop ? listing.instances : []),
    [desktop, listing.instances],
  );
  const skipped = useMemo(
    () => (desktop ? listing.skipped : []),
    [desktop, listing.skipped],
  );

  const visible = useMemo(
    () => sortInstances(filterInstances(instances, query)),
    [instances, query],
  );

  /** 按版本类型分组，顺序固定，空组不显示 —— 和 PCL 左侧的分类一致。 */
  const groups = useMemo(() => {
    const buckets = new Map<InstanceCategory, InstanceSummary[]>();
    for (const instance of visible) {
      const bucket = buckets.get(instance.category) ?? [];
      bucket.push(instance);
      buckets.set(instance.category, bucket);
    }
    return [...buckets.entries()].sort(
      (left, right) =>
        CATEGORY_ORDER.indexOf(left[0]) - CATEGORY_ORDER.indexOf(right[0]),
    );
  }, [visible]);

  const [deletingId, setDeletingId] = useState<string | null>(null);
  /** 折叠的分组；默认全部展开，未出现在集合里的就是展开。 */
  const [collapsedGroups, setCollapsedGroups] = useState<Set<InstanceCategory>>(
    () => new Set(),
  );
  const toggleGroup = useCallback((category: InstanceCategory) => {
    setCollapsedGroups((previous) => {
      const next = new Set(previous);
      if (next.has(category)) {
        next.delete(category);
      } else {
        next.add(category);
      }
      return next;
    });
  }, []);
  /** 当前选中要启动的实例：只给它显示一张启动面板，不再给 134 个实例各挂一张。 */
  const [selectedId, setSelectedId] = useState<string | null>(null);
  /** 选中的实例对象；筛选后它不在列表里时就不显示面板。 */
  const selected = useMemo(
    () => visible.find((instance) => instance.id === selectedId) ?? null,
    [visible, selectedId],
  );

  /**
   * 删实例记录。只删 Aether 的记录，`.minecraft` 里的游戏文件不动，所以提示里
   * 要写清楚这一点，免得用户以为会卸载游戏。
   */
  const handleDelete = useCallback(
    async (instance: InstanceSummary) => {
      setActionError(null);
      setNotice(null);
      setDeletingId(instance.id);
      try {
        await deleteInstance(instance.id);
        setNotice(
          `已删除实例「${instance.name}」。游戏文件仍在原 .minecraft 里，没有删除。`,
        );
        await reload();
      } catch (caught) {
        setActionError(
          caught instanceof Error ? caught.message : "删除失败",
        );
      } finally {
        setDeletingId(null);
      }
    },
    [reload],
  );

  const handleCreated = useCallback(
    (created: InstanceSummary) => {
      setListing((previous) => ({
        ...previous,
        instances: [...previous.instances, created],
      }));
    },
    [],
  );

  const handleImported = useCallback((imported: InstanceSummary[]) => {
    setListing((previous) => ({
      ...previous,
      instances: [...previous.instances, ...imported],
    }));
  }, []);

  // 徽章只报数量，空状态标题才负责「还没有实例」这句话，避免同一句文案出现两次
  const badgeText = instances.length === 0 ? "0 个实例" : summariseInstances(instances);

  return (
    <>
      <PageHeader
        title={page.label}
        description={page.summary}
        action={
          <Button
            appearance="primary"
            icon={<Add24Regular />}
            disabled={!desktop}
            onClick={() => setDialogOpen(true)}
          >
            创建实例
          </Button>
        }
      />

      {!desktop ? (
        <DesktopOnlyNotice body="浏览器预览读不到本机实例。用桌面版可以扫描、创建和启动实例。" />
      ) : null}

      {notice ? (
        <MessageBar intent="success">
          <MessageBarBody>{notice}</MessageBarBody>
        </MessageBar>
      ) : null}
      {actionError ? (
        <MessageBar intent="error">
          <MessageBarBody>{actionError}</MessageBarBody>
        </MessageBar>
      ) : null}

      <section className="status-panel list-panel" aria-labelledby="instances-heading">
        <div className="list-toolbar">
          <div className="toolbar-heading">
            <Text id="instances-heading" as="h2" size={400} weight="semibold">
              本机实例
            </Text>
            <Badge appearance="outline">{badgeText}</Badge>
          </div>

          <div className="toolbar-actions">
            <SearchBox
              className="filter-field"
              placeholder="按名称或版本筛选"
              aria-label="筛选实例"
              value={query}
              onChange={(_, data) => setQuery(data.value)}
            />
            <Button
              icon={<Add24Regular />}
              disabled={!desktop}
              aria-label="导入实例"
              onClick={() => setImportOpen(true)}
            >
              导入
            </Button>
            <Button
              icon={<ArrowSync24Regular />}
              onClick={reload}
              disabled={!desktop || status === "loading"}
            >
              刷新
            </Button>
          </div>
        </div>

        {status === "error" && error ? (
          <ErrorState
            title="读取实例失败"
            message={error}
            onRetry={reload}
            retryLabel="重新读取"
          />
        ) : status === "loading" && instances.length === 0 ? (
          <ListSkeleton rows={3} />
        ) : instances.length === 0 ? (
          <EmptyState
            icon={<BoxMultiple24Regular />}
            title="还没有实例"
            body={
              desktop
                ? "创建一个实例开始，或者从现有的 .minecraft、PCL2 导入。"
                : "浏览器预览里无法读取本机实例，打开桌面版即可看到列表。"
            }
            action={
              <Button
                appearance="primary"
                icon={<Add24Regular />}
                disabled={!desktop}
                onClick={() => setDialogOpen(true)}
              >
                创建实例
              </Button>
            }
          />
        ) : visible.length === 0 ? (
          <EmptyState
            icon={<Warning24Regular />}
            title="没有匹配的实例"
            body={`没有名称或版本包含「${query.trim()}」的实例，换个关键词试试。`}
          />
        ) : (
          <div className="instance-groups">
            {groups.map(([category, members]) => (
              <section className="instance-group" key={category}>
                <button
                  type="button"
                  className="instance-group-heading"
                  aria-expanded={!collapsedGroups.has(category)}
                  onClick={() => toggleGroup(category)}
                >
                  <ChevronRight24Regular
                    className={
                      collapsedGroups.has(category)
                        ? "group-chevron group-chevron-collapsed"
                        : "group-chevron"
                    }
                  />
                  <Text weight="semibold">{CATEGORY_LABEL[category]}</Text>
                  <Badge appearance="tint">{members.length}</Badge>
                </button>
                {collapsedGroups.has(category) ? null : (
                  <InstanceList
                    instances={members}
                    memoryMiB={memoryMiB}
                    javaLabel={javaKey === null ? null : formatJavaLabel(javaKey)}
                    selectedId={selectedId}
                    onSelect={(instance) => setSelectedId(instance.id)}
                    onDelete={(instance) => {
                      // 说清楚删的是什么，再问一次。删的是实例记录，不碰游戏文件。
                      const confirmed = window.confirm(
                        `删除实例「${instance.name}」？\n\n` +
                          "只删除 Aether 的实例记录。.minecraft 里的版本、存档、模组、投影都不会动。",
                      );
                      if (confirmed) {
                        void handleDelete(instance);
                      }
                    }}
                    deletingId={deletingId}
                  />
                )}
              </section>
            ))}
          </div>
        )}
        {/*
          启动面板：列表行本身很紧凑，`InstanceList` 也不承载额外内容，
          所以在列表下方按实例逐个挂面板，而不是塞进行内。
          面板是每个实例唯一的启动/停止入口。
        */}
        {/*
          启动面板只给**选中的那一个实例**。以前是给每个实例都挂一张，134 个实例
          就铺出 134 张卡，既慢又没法用。点击列表行来选中。
        */}
        {selected ? (
          <InstanceLaunchPanel
            key={selected.id}
            instanceId={selected.id}
            title={selected.name}
            initialState={selected.state}
          />
        ) : visible.length > 0 ? (
          <Text className="muted">
            点上面的实例来启动它。
          </Text>
        ) : null}

        {skipped.length > 0 ? (
          <div className="skipped-block">
            <Text weight="semibold">有 {skipped.length} 条记录没读出来</Text>
            <ul className="skipped-list">
              {skipped.map((item) => (
                <li key={item.id}>
                  <Text className="muted">
                    {item.id}：{describeSkippedReason(item.reason)}
                  </Text>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </section>

      <CreateInstanceDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onCreated={handleCreated}
      />

      <ImportInstancesDialog
        open={importOpen}
        onOpenChange={setImportOpen}
        onImported={handleImported}
      />

      <Text className="page-footnote muted">
        默认内存 {formatMemoryMiB(memoryMiB)}。当前仅支持启动官方 .minecraft 中已安装且已验证的现代版本（离线单人模式）；验证安装后再点击启动。
      </Text>
    </>
  );
}

/** `localStorage` 里存的是完整路径，这里只取文件名所在的最后一段。 */
function formatJavaLabel(key: string): string {
  const parts = key.split(/[\\/]/).filter((part) => part.length > 0);
  return parts.length > 0 ? parts[parts.length - 1] : key;
}
