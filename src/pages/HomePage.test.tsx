import { screen, waitFor } from "@testing-library/react";
import { renderWithState } from "../test/support";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InstanceListing, InstanceSummary } from "../ipc";
import { HomePage } from "./HomePage";

const instance: InstanceSummary = {
  id: "11111111-1111-4111-8111-111111111111",
  name: "1.21.4",
  version: "1.21.4",
  state: "ready",
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
  return renderWithState(<HomePage greeting="晚上好" summary="把实例管好" />);
}

describe("HomePage", () => {
  beforeEach(() => {
    vi.mocked(listInstances).mockResolvedValue(emptyListing);
  });

  it("有实例时主角区显示这个实例，而不是下载引导", async () => {
    vi.mocked(listInstances).mockResolvedValue({ instances: [instance], skipped: [] });
    renderHome();

    // 这是用户实际看到的 bug：134 个实例还写着「开始你的第一个实例」。
    await waitFor(() => {
      expect(screen.queryByText("开始你的第一个实例")).toBeNull();
    });
    expect(screen.getAllByText("Vanilla 1.21.4").length).toBeGreaterThan(0);
  });

  it("实例数量显示在状态栏里", async () => {
    vi.mocked(listInstances).mockResolvedValue({
      instances: [instance, { ...instance, id: "b", name: "b" }],
      skipped: [],
    });
    renderHome();

    await waitFor(() => {
      expect(screen.getByText("2 个")).toBeTruthy();
    });
  });

  it("一个实例都没有时显示下载引导", async () => {
    renderHome();

    expect(await screen.findByText("开始你的第一个实例")).toBeTruthy();
    expect(screen.getByText("还没有安装任何版本")).toBeTruthy();
  });
});
