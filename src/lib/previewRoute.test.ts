import { describe, expect, it } from "vitest";
import { parsePreviewLocation } from "./previewRoute";

/**
 * 独立预览窗口的片段解析。
 *
 * 片段由后端生成，但前端仍按不可信输入处理：缺字段、来源非法的一律当作
 * 「不是预览窗口」，否则会给 `read_schematic` 发畸形请求。
 */
describe("parsePreviewLocation", () => {
  it("reads a complete preview fragment", () => {
    const hash =
      "#/schematic-preview?instance=5f559290-a66f-4ed7-a799-9d4802fe4fa7" +
      "&path=%E5%BB%BA%E7%AD%91%2F%E7%A9%BA%E5%9F%8E%E8%AE%A1.litematic" +
      "&origin=version&version=1.21.4&title=%E7%A9%BA%E5%9F%8E%E8%AE%A1";

    expect(parsePreviewLocation(hash)).toEqual({
      instanceId: "5f559290-a66f-4ed7-a799-9d4802fe4fa7",
      relativePath: "建筑/空城计.litematic",
      origin: "version",
      originVersion: "1.21.4",
      title: "空城计",
    });
  });

  it("returns null for the normal app location", () => {
    expect(parsePreviewLocation("")).toBeNull();
    expect(parsePreviewLocation("#/schematics")).toBeNull();
  });

  it("rejects a fragment with no query", () => {
    expect(parsePreviewLocation("#/schematic-preview")).toBeNull();
  });

  it("rejects a missing instance or path", () => {
    expect(
      parsePreviewLocation("#/schematic-preview?path=a.litematic&origin=game"),
    ).toBeNull();
    expect(
      parsePreviewLocation("#/schematic-preview?instance=abc&origin=game"),
    ).toBeNull();
  });

  it("rejects an unknown origin", () => {
    expect(
      parsePreviewLocation(
        "#/schematic-preview?instance=abc&path=a.litematic&origin=elsewhere",
      ),
    ).toBeNull();
  });

  it("accepts every documented origin", () => {
    for (const origin of ["instance", "game", "version"]) {
      const parsed = parsePreviewLocation(
        `#/schematic-preview?instance=abc&path=a.litematic&origin=${origin}`,
      );
      expect(parsed?.origin).toBe(origin);
    }
  });
});
