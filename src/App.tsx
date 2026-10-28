import {
  FluentProvider,
  Text,
  webDarkTheme,
  webLightTheme,
} from "@fluentui/react-components";
import { useCallback, useEffect, useMemo, useState } from "react";
import "./App.css";
import { EmptyState } from "./components/EmptyState";
import { SchematicPreview } from "./components/SchematicPreview";
import { ErrorState, ListSkeleton } from "./components/PageState";
import { Sidebar } from "./components/Sidebar";
import {
  defaultApplicationInfo,
  getApplicationInfo,
  isTauri,
  readSchematic,
} from "./ipc";
import type {
  ApplicationInfo,
  SchematicDetail,
  SchematicOrigin,
} from "./ipc";
import { currentGreeting } from "./lib/greeting";
import { parsePreviewLocation } from "./lib/previewRoute";
import { useQuery } from "./lib/query";
import { useTheme } from "./lib/useTheme";
import { allPages } from "./navigation";
import type { PageId } from "./navigation";
import { AccountsPage } from "./pages/AccountsPage";
import { HomePage } from "./pages/HomePage";
import { InstancesPage } from "./pages/InstancesPage";
import { LibraryPage } from "./pages/LibraryPage";
import { PlaceholderPage } from "./pages/PlaceholderPage";
import { SchematicsPage } from "./pages/SchematicsPage";
import { SettingsPage } from "./pages/SettingsPage";

/**
 * 独立投影预览窗口。
 *
 * `open_schematic_window` 打开的窗口只带一个 `#/schematic-preview?…` 片段，
 * 里面只有实例 ID、相对路径、来源和标题 —— 没有路径，也没有脚本。窗口里
 * 渲染的就是下面这个组件，它按片段再问后端要那一个投影。
 */
function SchematicPreviewWindow({
  instanceId,
  relativePath,
  origin,
  originVersion,
  title,
}: {
  instanceId: string;
  relativePath: string;
  origin: SchematicOrigin;
  originVersion: string;
  title: string;
}) {
  const load = useCallback(
    () => readSchematic(instanceId, relativePath, origin, originVersion),
    [instanceId, relativePath, origin, originVersion],
  );
  const { status, data, error, reload } = useQuery<SchematicDetail>(load, true);

  return (
    <div className="preview-window">
      <div className="preview-window-heading">
        <Text as="h1" size={500} weight="semibold">
          {title || data?.metadata.name || "投影预览"}
        </Text>
        <Text className="muted" size={200}>
          拖动旋转 · 滚轮缩放 · 右键平移
        </Text>
      </div>
      {status === "loading" ? (
        <ListSkeleton />
      ) : status === "error" ? (
        <ErrorState
          title="打不开这个投影"
          message={error ?? "未知错误"}
          onRetry={reload}
        />
      ) : data ? (
        <>
          <SchematicPreview detail={data} />
          <Text className="muted preview-note">
            此投影出自 MC {data.metadata.createdForRelease ?? "未知版本"}
            （数据版本 {data.metadata.minecraftDataVersion}）
          </Text>
        </>
      ) : (
        <EmptyState
          icon={null}
          title="没有可显示的投影"
          body="这个文件里没有方块数据。"
        />
      )}
    </div>
  );
}

function App() {
  const [application, setApplication] =
    useState<ApplicationInfo>(defaultApplicationInfo);
  const [pageId, setPageId] = useState<PageId>("home");
  const [greeting] = useState(currentGreeting);
  const { preference, resolved, setPreference } = useTheme();

  // 独立预览窗口：整页只有预览，没有侧边栏。
  const preview = useMemo(
    () => parsePreviewLocation(window.location.hash),
    [],
  );

  useEffect(() => {
    if (!isTauri()) {
      return;
    }

    let cancelled = false;
    void getApplicationInfo()
      .then((info) => {
        if (!cancelled) {
          setApplication(info);
        }
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
    };
  }, []);

  const page = allPages.find((candidate) => candidate.id === pageId) ?? allPages[0];

  if (preview) {
    return (
      <FluentProvider theme={resolved === "dark" ? webDarkTheme : webLightTheme}>
        <div className="app-shell" data-theme={resolved}>
          <SchematicPreviewWindow {...preview} />
        </div>
      </FluentProvider>
    );
  }

  return (
    <FluentProvider theme={resolved === "dark" ? webDarkTheme : webLightTheme}>
      <div className="app-shell" data-theme={resolved}>
        <Sidebar
          application={application}
          current={pageId}
          onNavigate={setPageId}
        />
        <main className="workspace">
          {page.id === "home" ? (
            <HomePage
              application={application}
              greeting={greeting}
              summary={page.summary}
            />
          ) : page.id === "settings" ? (
            <SettingsPage
              page={page}
              themePreference={preference}
              onThemeChange={setPreference}
            />
          ) : page.id === "instances" ? (
            <InstancesPage page={page} />
          ) : page.id === "library" ? (
            <LibraryPage page={page} />
          ) : page.id === "schematics" ? (
            <SchematicsPage page={page} />
          ) : page.id === "accounts" ? (
            <AccountsPage page={page} />
          ) : (
            <PlaceholderPage page={page} />
          )}
        </main>
      </div>
    </FluentProvider>
  );
}

export default App;
