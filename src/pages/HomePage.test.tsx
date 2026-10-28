import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InstanceListing, InstanceSummary } from "../ipc";
import { HomePage } from "./HomePage";

const instance: InstanceSummary = {
  id: "11111111-1111-4111-8111-111111111111",
  name: "1.21.4",
  version: "1.21.4",
  state: "ready",
  category: "vanilla",
  gameDirectory: "C:\\Games\\.minecraft",
};

const emptyListing: InstanceListing = { instances: [], skipped: [] };

vi.mock("../ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../ipc")>();
  return {
    ...actual,
    isTauri: () => true,
    listInstances: vi.fn(async () => emptyListing),
    detectLauncherInstallations: vi.fn(async () => []),
  };
});

const { listInstances } = await import("../ipc");

function renderHome() {
  return render(
    <HomePage
      application={{ coreStatus: "ready" } as never}
      greeting="晚上好"
      summary="把实例管好"
      onImport={() => undefined}
      onOpenInstances={() => undefined}
    />,
  );
}

describe("HomePage", () => {
  beforeEach(() => {
    vi.mocked(listInstances).mockResolvedValue(emptyListing);
  });

  it("已经有实例时不再显示「尚未导入实例」", async () => {
    vi.mocked(listInstances).mockResolvedValue({ instances: [instance], skipped: [] });
    renderHome();

    // 这是用户实际看到的 bug：134 个实例还写着「尚未导入」。
    await waitFor(() => {
      expect(screen.queryByText("尚未导入实例")).toBeNull();
    });
    expect(screen.getByRole("heading", { name: "1.21.4" })).toBeTruthy();
    expect(screen.getByText("1 个实例")).toBeTruthy();
  });

  it("实例数量显示在环境检查里", async () => {
    vi.mocked(listInstances).mockResolvedValue({
      instances: [instance, { ...instance, id: "b", name: "b" }],
      skipped: [],
    });
    renderHome();

    await waitFor(() => {
      expect(screen.getByText("2 个实例")).toBeTruthy();
    });
  });

  it("一个实例都没有时才显示导入引导，且启动按钮不可用", async () => {
    renderHome();

    expect(await screen.findByText("尚未导入实例")).toBeTruthy();
    expect(screen.getByRole("button", { name: "启动" })).toBeDisabled();
  });
});
