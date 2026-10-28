import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AssetPack, AssetPackCandidate } from "../ipc";
import { AssetPackPicker } from "./AssetPackPicker";
import { materialStorageKey } from "../lib/schematicMaterial";

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

const { listAssetPackCandidates, loadAssetPack } = await import("../ipc");

describe("AssetPackPicker", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.mocked(listAssetPackCandidates).mockResolvedValue(candidates);
    vi.mocked(loadAssetPack).mockClear();
  });

  it("没设置过时提示可以选版本，并默认选中已提取过的那个", async () => {
    render(
      <AssetPackPicker
        pack={null}
        onLoaded={() => undefined}
        onPreferenceChange={() => undefined}
      />,
    );

    expect(await screen.findByText(/当前是纯色预览/)).toBeTruthy();
    await waitFor(() => {
      expect(
        screen.getByRole("combobox", { name: "选择用来提取材质的版本" }),
      ).toHaveProperty("value", "1.21.4");
    });
  });

  it("点「使用材质」后把选择保存下来，下次不再询问", async () => {
    const onLoaded = vi.fn();
    const onPreferenceChange = vi.fn();
    render(
      <AssetPackPicker
        pack={null}
        onLoaded={onLoaded}
        onPreferenceChange={onPreferenceChange}
      />,
    );

    fireEvent.click(await screen.findByRole("button", { name: "使用材质" }));

    await waitFor(() => {
      expect(onLoaded).toHaveBeenCalledTimes(1);
    });
    // 关键：选择写进存储，用户不用每次进预览重新挑。
    expect(window.localStorage.getItem(materialStorageKey)).toBe("1.21.4");
    expect(onPreferenceChange).toHaveBeenCalledWith("1.21.4");
  });

  it("已经设置过且有资源包时整条提示都不显示", () => {
    window.localStorage.setItem(materialStorageKey, "1.21.4");
    render(
      <AssetPackPicker
        pack={{ sourceVersion: "1.21.4" } as AssetPack}
        onLoaded={() => undefined}
        onPreferenceChange={() => undefined}
      />,
    );

    expect(screen.queryByText(/当前是纯色预览/)).toBeNull();
  });

  it("一个候选版本都没有时说明原因，不给按钮", async () => {
    vi.mocked(listAssetPackCandidates).mockResolvedValue([]);
    render(
      <AssetPackPicker
        pack={null}
        onLoaded={() => undefined}
        onPreferenceChange={() => undefined}
      />,
    );

    expect(await screen.findByText(/没有找到客户端 jar/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "使用材质" })).toBeNull();
  });
});
