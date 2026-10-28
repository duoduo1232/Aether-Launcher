import { invoke } from "@tauri-apps/api/core";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { InstanceListing, SchematicListing } from "../ipc";
import { SchematicsPage } from "./SchematicsPage";
import type { PageDefinition } from "../navigation";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const page: PageDefinition = {
  id: "schematics",
  label: "投影图",
  icon: <span />,
  summary: "Litematica 投影文件的 3D 预览与跨版本管理",
  pendingTitle: "",
  pendingBody: "",
};

const SURVIVAL = "11111111-1111-4111-8111-111111111111";
const REDSTONE = "22222222-2222-4222-8222-222222222222";

/**
 * 每个版本标题上都有一组同名按钮（导入投影 / 导入文件夹），按名字取第一个 ——
 * 测试关心的是「点了会怎样」，不是点的是哪个版本。
 */
function firstByRole(role: string, name: string): HTMLElement {
  return screen.getAllByRole(role, { name })[0];
}

function metadata(name: string) {
  return {
    name,
    author: "tester",
    description: "",
    formatVersion: 6,
    minecraftDataVersion: 3955,
    createdForRelease: "1.21.1",
    regionCount: 1,
    totalBlocks: 800,
    totalVolume: 1000,
    enclosingSize: [10, 10, 10] as [number, number, number],
  };
}

/** 后端只返回有投影的版本；`空版本` 只出现在实例列表里。 */
function schematicListing(): SchematicListing {
  return {
    instances: [
      {
        id: SURVIVAL,
        name: "生存 1.21",
        version: "1.21.1",
        state: "ready",
        schematics: [
          {
            relativePath: "castle.litematic",
            fileName: "castle.litematic",
            sizeBytes: 2048,
            origin: "instance",
            originVersion: "",
            metadata: metadata("城堡"),
          },
          {
            relativePath: "红石/circuit.litematic",
            fileName: "circuit.litematic",
            sizeBytes: 4096,
            origin: "instance",
            originVersion: "",
            metadata: metadata("红石电路"),
          },
        ],
        failed: [],
        sources: [
          { origin: "instance", originVersion: "", label: "实例目录/schematics" },
        ],
      },
      {
        id: REDSTONE,
        name: "红石实验",
        version: "1.20.1",
        state: "ready",
        schematics: [
          {
            relativePath: "trap.litematic",
            fileName: "trap.litematic",
            sizeBytes: 1024,
            origin: "version",
            originVersion: "1.20.1",
            metadata: metadata("陷阱"),
          },
        ],
        failed: [],
        sources: [
          { origin: "version", originVersion: "1.20.1", label: "versions/1.20.1/schematics" },
        ],
      },
    ],
    skipped: [],
    missingGameDirectory: 0,
  };
}

function instanceListing(): InstanceListing {
  return {
    instances: [
      // 实例列表里的游戏目录：老记录为 null。
      {
        id: SURVIVAL,
        name: "生存 1.21",
        version: "1.21.1",
        state: "ready",
        category: "vanilla",
        gameDirectory: null,
      },
      {
        id: REDSTONE,
        name: "红石实验",
        version: "1.20.1",
        state: "ready",
        category: "vanilla",
        gameDirectory: null,
      },
      {
        id: "33333333-3333-4333-8333-333333333333",
        name: "空版本",
        version: "1.19.2",
        state: "ready",
        category: "vanilla",
        gameDirectory: null,
      },
    ],
    skipped: [],
  };
}

function enableTauri() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    value: {},
    configurable: true,
  });
}

function mockBackend(
  overrides: Record<string, (args: unknown) => unknown> = {},
) {
  vi.mocked(invoke).mockImplementation(
    async (command: string, args?: unknown) => {
      const handler = overrides[command];
      if (handler) {
        return handler(args);
      }
      if (command === "list_schematics") {
        return schematicListing();
      }
      if (command === "list_instances") {
        return instanceListing();
      }
      throw new Error(`unexpected command ${command}`);
    },
  );
}

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

afterEach(() => {
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  vi.restoreAllMocks();
});

describe("SchematicsPage", () => {
  it("renders the desktop notice in a plain browser", () => {
    render(<SchematicsPage page={page} />);

    expect(
      screen.getByRole("heading", { level: 1, name: "投影图" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/浏览器预览读不到本机实例/)).toBeInTheDocument();
  });

  it("only shows versions that actually have schematics", async () => {
    enableTauri();
    mockBackend();

    render(<SchematicsPage page={page} />);

    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });
    expect(
      screen.getByRole("button", { name: "生存 1.21（1.21.1）" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "红石实验（1.20.1）" }),
    ).toBeInTheDocument();
    // `空版本` 没有 schematics 目录，后端不返回，页面也就不该出现。
    expect(screen.queryByText("空版本")).not.toBeInTheDocument();
  });

  it("keeps the folder structure under each version", async () => {
    enableTauri();
    mockBackend();

    render(<SchematicsPage page={page} />);

    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });
    // 文件夹默认折叠：根上的文件直接可见，文件夹里的要展开才渲染。
    const folder = screen.getByRole("button", { name: "文件夹 红石" });
    expect(folder).toBeInTheDocument();
    expect(screen.queryByText("红石电路")).not.toBeInTheDocument();

    fireEvent.click(folder);
    expect(screen.getByText("红石电路")).toBeInTheDocument();
  });

  it("expands a collapsed version on click", async () => {
    enableTauri();
    mockBackend();

    render(<SchematicsPage page={page} />);

    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });
    // 第二个版本默认折叠。
    expect(screen.queryByText("陷阱")).not.toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: "红石实验（1.20.1）" }),
    );

    expect(await screen.findByText("陷阱")).toBeInTheDocument();
  });

  it("copies a schematic into the clipboard from the context menu", async () => {
    enableTauri();
    mockBackend();

    render(<SchematicsPage page={page} />);
    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });

    fireEvent.contextMenu(screen.getByRole("button", { name: "城堡" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "复制" }));

    expect(
      await screen.findByText(/已复制 1 个投影：castle\.litematic/),
    ).toBeInTheDocument();
  });

  it("pastes into a folder through the context menu", async () => {
    enableTauri();
    mockBackend();

    render(<SchematicsPage page={page} />);
    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });

    fireEvent.contextMenu(screen.getByRole("button", { name: "城堡" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "复制" }));
    await screen.findByText(/已复制 1 个投影：castle\.litematic/);

    fireEvent.contextMenu(
      screen.getByRole("button", { name: "文件夹 红石" }),
    );
    fireEvent.click(await screen.findByRole("menuitem", { name: "粘贴到此处" }));

    await waitFor(() => {
      expect(vi.mocked(invoke)).toHaveBeenCalledWith("paste_schematic", {
        fromInstanceId: SURVIVAL,
        fromPath: "castle.litematic",
        fromOrigin: "instance",
        fromOriginVersion: "",
        toInstanceId: SURVIVAL,
        toFolder: "红石",
        moveFile: false,
      });
    });
  });

  it("deletes a schematic after confirmation", async () => {
    enableTauri();
    mockBackend();
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<SchematicsPage page={page} />);
    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });

    fireEvent.contextMenu(screen.getByRole("button", { name: "城堡" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "删除" }));

    await waitFor(() => {
      expect(vi.mocked(invoke)).toHaveBeenCalledWith("delete_schematic", {
        instanceId: SURVIVAL,
        relativePath: "castle.litematic",
        origin: "instance",
        originVersion: "",
      });
    });
  });

  it("starts multi-select mode from the context menu", async () => {
    enableTauri();
    mockBackend();

    render(<SchematicsPage page={page} />);
    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });

    fireEvent.contextMenu(screen.getByRole("button", { name: "城堡" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "多选" }));

    expect(await screen.findByText("已选 1 项")).toBeInTheDocument();
    expect(
      screen.getByRole("checkbox", { name: "选择 castle.litematic" }),
    ).toBeChecked();
  });

  it("reports unparseable files inside the owning version", async () => {
    enableTauri();
    mockBackend({
      list_schematics: () => {
        const listing = schematicListing();
        listing.instances[0].schematics = [];
        listing.instances[0].failed = [
          {
            relativePath: "城堡/broken.litematic",
            fileName: "broken.litematic",
            reason: "unparseable",
            origin: "instance",
            originVersion: "",
          },
        ];
        return listing;
      },
    });

    render(<SchematicsPage page={page} />);

    await waitFor(() => {
      expect(screen.getByText(/1 个文件无法解析/)).toBeInTheDocument();
    });
    expect(screen.getByText(/城堡\/broken\.litematic/)).toBeInTheDocument();
  });

  it("explains the empty state when no version has schematics", async () => {
    enableTauri();
    mockBackend({
      list_schematics: () => ({ instances: [], skipped: [] }),
    });

    render(<SchematicsPage page={page} />);

    await waitFor(() => {
      expect(screen.getByText("还没有版本带投影文件")).toBeInTheDocument();
    });
  });

  it("shows an import error for wrong extensions without calling the backend", async () => {
    enableTauri();
    mockBackend();

    render(<SchematicsPage page={page} />);
    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });

    fireEvent.click(firstByRole("button", "导入投影"));

    const input = document.querySelector('input[type="file"]');
    expect(input).not.toBeNull();
    Object.defineProperty(input, "value", { writable: true });
    Object.defineProperty(input, "files", {
      value: [new File(["data"], "not-a-schematic.zip")],
      configurable: true,
    });
    fireEvent.change(input as Element);

    expect(await screen.findByText(/只能导入 \.litematic 文件/)).toBeInTheDocument();
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
      "import_schematic",
      expect.anything(),
    );
  });

  it("imports a whole folder and reports what was skipped", async () => {
    enableTauri();
    mockBackend({
      import_schematic_folder: () => ({
        imported: 3,
        skipped: [{ relativePath: "broken.litematic", reason: "unparseable" }],
      }),
    });

    render(<SchematicsPage page={page} />);
    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });

    fireEvent.click(firstByRole("button", "导入文件夹"));

    expect(
      await screen.findByText(/已导入 3 个投影，跳过 1 个/),
    ).toBeInTheDocument();
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("import_schematic_folder", {
      instanceId: expect.any(String),
    });
  });

  it("does nothing when the folder picker is dismissed", async () => {
    enableTauri();
    mockBackend({ import_schematic_folder: () => null });

    render(<SchematicsPage page={page} />);
    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });

    fireEvent.click(firstByRole("button", "导入文件夹"));

    await waitFor(() =>
      expect(vi.mocked(invoke)).toHaveBeenCalledWith(
        "import_schematic_folder",
        expect.anything(),
      ),
    );
    expect(screen.queryByText(/已导入/)).not.toBeInTheDocument();
  });

  it("re-reads both listings when refresh is clicked", async () => {
    enableTauri();
    mockBackend();

    render(<SchematicsPage page={page} />);
    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });

    const callsBefore = vi
      .mocked(invoke)
      .mock.calls.filter(([command]) => command === "list_schematics").length;

    fireEvent.click(screen.getByRole("button", { name: "刷新" }));

    await waitFor(() => {
      const callsAfter = vi
        .mocked(invoke)
        .mock.calls.filter(([command]) => command === "list_schematics").length;
      expect(callsAfter).toBeGreaterThan(callsBefore);
    });
    // 实例下拉框也要更新：别处新建的版本得能选到。
    expect(
      vi.mocked(invoke).mock.calls.filter(([command]) => command === "list_instances")
        .length,
    ).toBeGreaterThan(1);
  });
});
