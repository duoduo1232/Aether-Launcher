import { invoke } from "@tauri-apps/api/core";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SchematicListing } from "../ipc";
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

function enableTauri() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    value: {},
    configurable: true,
  });
}

function instanceListing(): SchematicListing {
  return {
    instances: [
      {
        id: "11111111-1111-4111-8111-111111111111",
        name: "生存 1.21",
        version: "1.21.1",
        state: "ready",
        schematics: [
          {
            fileName: "castle.litematic",
            sizeBytes: 2048,
            metadata: {
              name: "城堡",
              author: "tester",
              description: "",
              formatVersion: 6,
              minecraftDataVersion: 3955,
              createdForRelease: "1.21.1",
              regionCount: 1,
              totalBlocks: 800,
              totalVolume: 1000,
              enclosingSize: [10, 10, 10],
            },
          },
        ],
        failed: [],
      },
      {
        id: "22222222-2222-4222-8222-222222222222",
        name: "红石实验",
        version: "1.20.1",
        state: "ready",
        schematics: [],
        failed: [],
      },
    ],
    skipped: [],
  };
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

    expect(screen.getByRole("heading", { level: 1, name: "投影图" })).toBeInTheDocument();
    expect(screen.getByText(/浏览器预览读不到本机实例/)).toBeInTheDocument();
  });

  it("groups automatically discovered schematics by instance", async () => {
    enableTauri();
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "list_schematics") {
        return instanceListing();
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<SchematicsPage page={page} />);

    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });
    expect(screen.getByText("生存 1.21")).toBeInTheDocument();
    expect(screen.getByText(/出自 MC 1.21.1/)).toBeInTheDocument();
    // 空实例给出说明而不是被折叠。
    expect(
      screen.getByText(/这个实例的 schematics 目录里还没有投影文件/),
    ).toBeInTheDocument();
  });

  it("reports unparseable files inside the owning instance", async () => {
    enableTauri();
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "list_schematics") {
        const listing = instanceListing();
        listing.instances[0].schematics = [];
        listing.instances[0].failed = [
          { fileName: "broken.litematic", reason: "unparseable" },
        ];
        return listing;
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<SchematicsPage page={page} />);

    await waitFor(() => {
      expect(screen.getByText(/1 个文件无法解析/)).toBeInTheDocument();
    });
    expect(screen.getByText(/broken\.litematic/)).toBeInTheDocument();
  });

  it("shows an import error for wrong extensions without calling the backend", async () => {
    enableTauri();
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "list_schematics") {
        return instanceListing();
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<SchematicsPage page={page} />);
    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });

    // 先选定导入目标实例，再触发文件选择。
    fireEvent.click(screen.getByRole("button", { name: /导入到第一个实例/ }));

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
});
