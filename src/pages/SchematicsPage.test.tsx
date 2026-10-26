import { invoke } from "@tauri-apps/api/core";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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
    expect(screen.getByText(/浏览器预览读不到本机投影图/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "导入投影图" })).toBeDisabled();
  });

  it("lists schematics returned by the backend", async () => {
    enableTauri();
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "list_schematics") {
        return {
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
        };
      }
      if (command === "list_instances") {
        return { instances: [], skipped: [] };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<SchematicsPage page={page} />);

    await waitFor(() => {
      expect(screen.getByText("城堡")).toBeInTheDocument();
    });
    expect(screen.getByText(/出自 MC 1.21.1/)).toBeInTheDocument();
    expect(screen.getByText(/2\.0 KB/)).toBeInTheDocument();
  });

  it("reports unparseable files without hiding the good ones", async () => {
    enableTauri();
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "list_schematics") {
        return {
          schematics: [],
          failed: [{ fileName: "broken.litematic", reason: "unparseable" }],
        };
      }
      if (command === "list_instances") {
        return { instances: [], skipped: [] };
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
        return { schematics: [], failed: [] };
      }
      if (command === "list_instances") {
        return { instances: [], skipped: [] };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<SchematicsPage page={page} />);
    await waitFor(() => {
      expect(screen.getByText("还没有投影图")).toBeInTheDocument();
    });

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
