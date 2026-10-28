import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AssetPack, AssetPackCandidate } from "../ipc";
import { AssetPackPicker } from "./AssetPackPicker";

const candidates: AssetPackCandidate[] = [
  { version: "1.20.1", sizeMb: 24, extracted: false },
  { version: "1.21.4", sizeMb: 26, extracted: true },
];

vi.mock("../ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../ipc")>();
  return {
    ...actual,
    listAssetPackCandidates: vi.fn(async () => candidates),
    loadAssetPack: vi.fn(
      async (version: string) => ({ sourceVersion: version }) as AssetPack,
    ),
  };
});

const { listAssetPackCandidates } = await import("../ipc");

describe("AssetPackPicker", () => {
  beforeEach(() => {
    vi.mocked(listAssetPackCandidates).mockResolvedValue(candidates);
  });

  it("没有资源包时提示可以选版本，并且默认选中已经提取过的那个", async () => {
    render(<AssetPackPicker pack={null} onLoaded={() => undefined} />);

    expect(await screen.findByText("当前是纯色预览。")).toBeTruthy();
    // 默认选中已提取的版本，用户直接点按钮就能用上缓存。
    await waitFor(() => {
      expect(screen.getByRole("combobox", { name: "选择用来提取材质的版本" })).toHaveProperty(
        "value",
        "1.21.4",
      );
    });
  });

  it("已经有资源包时整条提示都不显示", () => {
    render(
      <AssetPackPicker
        pack={{ sourceVersion: "1.21.4" } as AssetPack}
        onLoaded={() => undefined}
      />,
    );

    expect(screen.queryByText("当前是纯色预览。")).toBeNull();
  });

  it("一个候选版本都没有时说明原因，不给按钮", async () => {
    vi.mocked(listAssetPackCandidates).mockResolvedValue([]);
    render(<AssetPackPicker pack={null} onLoaded={() => undefined} />);

    expect(await screen.findByText(/没有找到客户端 jar/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "提取材质" })).toBeNull();
  });
});
