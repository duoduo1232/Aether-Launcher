import {
  Badge,
  Button,
  ProgressBar,
  Spinner,
  Text,
  Tooltip,
} from "@fluentui/react-components";
import {
  Add24Regular,
  ArrowSync24Regular,
  Person24Regular,
  Play24Filled,
} from "@fluentui/react-icons";
import { useCallback, useEffect, useState } from "react";
import { PageHeader } from "../components/PageHeader";
import type { ApplicationInfo, InstanceSummary } from "../ipc";
import { detectLauncherInstallations, isTauri, listInstances } from "../ipc";
import { summariseScan } from "../lib/launcherScan";
import type { LauncherScan } from "../lib/launcherScan";

type HomePageProps = {
  application: ApplicationInfo;
  greeting: string;
  summary: string;
  /** 打开导入对话框。 */
  onImport: () => void;
  /** 跳到实例页。 */
  onOpenInstances: () => void;
};

/**
 * 首页。
 *
 * 「导入实例」打开和实例页同一个导入对话框；「扫描本机」跑一次启动器探测。
 *
 * 这里显示的是**真实**的实例数据：有实例时标题显示最近一个实例的名字、
 * 徽章显示实例总数，卡片右上方的启动按钮跳到实例页启动它。没有实例时才
 * 显示导入引导。
 */
export function HomePage({
  application,
  greeting,
  summary,
  onImport,
  onOpenInstances,
}: HomePageProps) {
  const [scan, setScan] = useState<LauncherScan | null>(null);
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [instances, setInstances] = useState<InstanceSummary[]>([]);
  const desktop = isTauri();

  // 首页要反映「已经有多少个实例」，不能一直写死成「尚未导入实例」。
  useEffect(() => {
    if (!desktop) {
      return;
    }
    let cancelled = false;
    void listInstances()
      .then((listing) => {
        if (!cancelled) {
          setInstances(listing.instances);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setInstances([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [desktop]);

  useEffect(() => {
    // 进首页时静默探一次，用户不必先点按钮才知道本机有没有启动器。
    if (!desktop) {
      return;
    }
    let cancelled = false;
    void detectLauncherInstallations()
      .then((result) => {
        if (!cancelled) {
          setScan(summariseScan(result));
        }
      })
      .catch(() => {
        if (!cancelled) {
          setScan(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [desktop]);

  const runScan = useCallback(async () => {
    setScanning(true);
    setScanError(null);
    try {
      setScan(summariseScan(await detectLauncherInstallations()));
    } catch (cause) {
      setScanError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setScanning(false);
    }
  }, []);

  const featured = instances[0] ?? null;

  return (
    <>
      <PageHeader
        title={greeting}
        description={summary}
        action={
          <Button appearance="subtle" icon={<Person24Regular />}>
            添加账户
          </Button>
        }
      />

      <section className="launch-surface" aria-labelledby="current-instance">
        <div className="instance-art" aria-hidden="true">
          <span>{featured ? featured.version : "—"}</span>
        </div>
        <div className="instance-copy">
          <div className="instance-heading">
            <Text id="current-instance" as="h2" size={500} weight="semibold">
              {featured ? featured.name : "尚未导入实例"}
            </Text>
            {instances.length > 0 ? (
              <Badge appearance="tint" color="success">
                {instances.length} 个实例
              </Badge>
            ) : (
              <Badge appearance="tint" color="informative">
                早期开发
              </Badge>
            )}
          </div>
          <Text className="muted">
            {featured
              ? `版本 ${featured.version}。在实例页里可以启动、改设置或删除。`
              : "从现有 .minecraft 或其他启动器导入，Aether 只补齐缺失内容"}
          </Text>
          {/* 已经有实例时不再占版面：导入入口在实例页的页头，首页只留启动。 */}
          {instances.length === 0 ? (
            <div className="instance-actions">
              <Button
                appearance="primary"
                icon={<Add24Regular />}
                onClick={onImport}
              >
                导入实例
              </Button>
              <Button
                icon={<ArrowSync24Regular />}
                onClick={() => void runScan()}
                disabled={scanning}
              >
                扫描本机
              </Button>
              {scanning ? <Spinner size="tiny" /> : null}
              {scan ? (
                <Text className="muted instance-scan-note">
                  找到 {scan.installations.length} 个启动器，{scan.totalVersions} 个版本
                  {scan.installations.length > 0 ? (
                    <Button appearance="transparent" size="small" onClick={onOpenInstances}>
                      去导入
                    </Button>
                  ) : null}
                </Text>
              ) : null}
              {scanError ? (
                <Text className="muted instance-scan-note">扫描失败：{scanError}</Text>
              ) : null}
              {!desktop ? (
                <Text className="muted instance-scan-note">
                  浏览器预览里读不到本机启动器，打开桌面版即可扫描。
                </Text>
              ) : null}
            </div>
          ) : null}
        </div>
        <Tooltip
          content={featured ? "在实例页启动这个实例" : "导入实例后即可启动"}
          relationship="description"
        >
          <Button
            className="launch-button"
            appearance="primary"
            icon={<Play24Filled />}
            size="large"
            disabled={!featured}
            onClick={onOpenInstances}
          >
            启动
          </Button>
        </Tooltip>
      </section>

      <section className="status-grid" aria-label="启动器状态">
        <article className="status-panel">
          <div className="panel-heading">
            <Text as="h2" size={400} weight="semibold">
              环境检查
            </Text>
            <Badge appearance="outline">待扫描</Badge>
          </div>
          <dl className="status-list">
            <div>
              <dt>Java 运行时</dt>
              <dd>尚未检测</dd>
            </div>
            <div>
              <dt>游戏实例</dt>
              <dd>{instances.length}</dd>
            </div>
            <div>
              <dt>核心阶段</dt>
              <dd>{application.coreStatus}</dd>
            </div>
          </dl>
        </article>

        <article className="status-panel">
          <div className="panel-heading">
            <Text as="h2" size={400} weight="semibold">
              下载队列
            </Text>
            <Text className="muted">0 项</Text>
          </div>
          <ProgressBar value={0} />
          <div className="empty-state">
            <Text weight="semibold">没有进行中的任务</Text>
            <Text className="muted">
              下载计划将显示来源、校验值与失败原因
            </Text>
          </div>
        </article>
      </section>
    </>
  );
}
