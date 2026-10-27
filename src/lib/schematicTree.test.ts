import { describe, expect, it } from "vitest";
import type { SchematicOrigin, SchematicSummary } from "../ipc";
import {
  buildSchematicSources,
  buildSchematicTree,
  collectFilePaths,
  collectFiles,
  sourceLabel,
} from "./schematicTree";

function summary(
  relativePath: string,
  origin: SchematicOrigin = "instance",
  originVersion = "",
): SchematicSummary {
  const fileName = relativePath.split("/").pop() ?? relativePath;
  return {
    relativePath,
    fileName,
    sizeBytes: 1024,
    origin,
    originVersion,
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

describe("buildSchematicSources", () => {
  it("分开了同一个相对路径出现在两个 schematics 的情况", () => {
    // 实例自己的目录和 1.20.1 版本的 schematics 里都有 a.litematic：
    // 这是两个不同文件，不能合并成一条。
    const sources = buildSchematicSources([
      summary("a.litematic", "instance", ""),
      summary("a.litematic", "version", "1.20.1"),
    ]);

    expect(sources).toHaveLength(2);
    const instance = sources.find((item) => item.origin === "instance");
    const version = sources.find((item) => item.origin === "version");
    expect(instance?.root.files.map((file) => file.fileName)).toEqual(["a.litematic"]);
    expect(version?.root.files.map((file) => file.fileName)).toEqual(["a.litematic"]);
    expect(version?.label).toBe("versions/1.20.1/schematics");
  });

  it("保留了来源在树里的子文件夹结构", () => {
    const sources = buildSchematicSources([
      summary("建筑/空城计.litematic", "game", ""),
      summary("建筑/实装樱花小屋.litematic", "game", ""),
    ]);

    expect(sources).toHaveLength(1);
    expect(sources[0].label).toBe("schematics");
    expect(sources[0].root.folders.map((folder) => folder.name)).toEqual(["建筑"]);
    expect(sources[0].total).toBe(2);
  });

  it("collectFiles 抓得到子文件夹里的文件并带上来源", () => {
    const sources = buildSchematicSources([
      summary("建筑/空城计.litematic", "game", ""),
      summary("root.litematic", "instance", ""),
    ]);
    const gameSource = sources.find((item) => item.origin === "game");

    const files = collectFiles(gameSource!.root);
    expect(files.map((file) => file.fileName)).toEqual(["空城计.litematic"]);
    expect(files[0].origin).toBe("game");
  });

  it("sourceLabel 与后端保持同一套名字", () => {
    expect(sourceLabel("instance", "")).toBe("实例目录/schematics");
    expect(sourceLabel("game", "")).toBe("schematics");
    expect(sourceLabel("version", "1.21.4")).toBe("versions/1.21.4/schematics");
  });
});
