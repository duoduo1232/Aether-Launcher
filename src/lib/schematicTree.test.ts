import { describe, expect, it } from "vitest";
import type { SchematicSummary } from "../ipc";
import { buildSchematicTree, collectFilePaths } from "./schematicTree";

function summary(relativePath: string): SchematicSummary {
  const fileName = relativePath.split("/").pop() ?? relativePath;
  return {
    relativePath,
    fileName,
    sizeBytes: 1024,
    metadata: {
      name: fileName,
      author: "",
      description: "",
      formatVersion: 6,
      minecraftDataVersion: 3955,
      createdForRelease: "1.21.1",
      regionCount: 1,
      totalBlocks: 1,
      totalVolume: 1,
      enclosingSize: [1, 1, 1],
    },
  };
}

describe("buildSchematicTree", () => {
  it("keeps root files at the root", () => {
    const root = buildSchematicTree([summary("castle.litematic")]);

    expect(root.path).toBe("");
    expect(root.folders).toEqual([]);
    expect(root.files.map((file) => file.fileName)).toEqual(["castle.litematic"]);
    expect(root.total).toBe(1);
  });

  it("rebuilds nested folders from relative paths", () => {
    const root = buildSchematicTree([
      summary("城堡/主楼/deep.litematic"),
      summary("城堡/塔.litematic"),
      summary("红石/circuit.litematic"),
      summary("root.litematic"),
    ]);

    expect(root.folders.map((folder) => folder.name)).toEqual(["城堡", "红石"]);
    const castle = root.folders[0];
    expect(castle.path).toBe("城堡");
    expect(castle.folders.map((folder) => folder.path)).toEqual(["城堡/主楼"]);
    expect(castle.files.map((file) => file.fileName)).toEqual(["塔.litematic"]);
    expect(castle.folders[0].files.map((file) => file.fileName)).toEqual([
      "deep.litematic",
    ]);
    expect(root.files.map((file) => file.fileName)).toEqual(["root.litematic"]);
  });

  it("counts descendants in the folder total", () => {
    const root = buildSchematicTree([
      summary("城堡/主楼/deep.litematic"),
      summary("城堡/塔.litematic"),
      summary("红石/circuit.litematic"),
      summary("root.litematic"),
    ]);

    expect(root.total).toBe(4);
    expect(root.folders[0].total).toBe(2);
    expect(root.folders[0].folders[0].total).toBe(1);
    expect(root.folders[1].total).toBe(1);
  });

  it("sorts folders before files and both by name", () => {
    const root = buildSchematicTree([
      summary("b.litematic"),
      summary("a.litematic"),
      summary("zeta/x.litematic"),
      summary("alpha/y.litematic"),
    ]);

    expect(root.folders.map((folder) => folder.name)).toEqual(["alpha", "zeta"]);
    expect(root.files.map((file) => file.fileName)).toEqual([
      "a.litematic",
      "b.litematic",
    ]);
  });

  it("treats a single file as a root entry", () => {
    const root = buildSchematicTree([summary("only.litematic")]);
    expect(collectFilePaths(root)).toEqual(["only.litematic"]);
  });

  it("collects every file under a folder", () => {
    const root = buildSchematicTree([
      summary("城堡/主楼/deep.litematic"),
      summary("城堡/塔.litematic"),
      summary("红石/circuit.litematic"),
    ]);

    expect(collectFilePaths(root.folders[0]).sort()).toEqual([
      "城堡/主楼/deep.litematic",
      "城堡/塔.litematic",
    ]);
    expect(collectFilePaths(root).sort()).toEqual([
      "城堡/主楼/deep.litematic",
      "城堡/塔.litematic",
      "红石/circuit.litematic",
    ]);
  });

  it("returns an empty root for no files", () => {
    const root = buildSchematicTree([]);
    expect(root.folders).toEqual([]);
    expect(root.files).toEqual([]);
    expect(root.total).toBe(0);
  });
});
