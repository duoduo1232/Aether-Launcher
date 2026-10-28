import { Dropdown, Option, Spinner, Text } from "@fluentui/react-components";
import { useCallback, useEffect, useState } from "react";
import type { AssetPackCandidate } from "../ipc";
import { listAssetPackCandidates } from "../ipc";
import {
  loadMaterialPreference,
  saveMaterialPreference,
} from "../lib/schematicMaterial";

/**
 * 设置页的「投影图材质」。
 *
 * 这里只选择**从哪个版本取材质**，不做提取 —— 真正的提取发生在第一次打开
 * 投影预览时（按需），选中就保存。选「纯色预览」则清空设置。
 *
 * 为什么不做成打开预览就自动提取：一个客户端 jar 二十多 MB、上千个 model，
 * 进页面就解压要等好几秒。提取结果按 jar（路径 + 大小 + 修改时间）缓存在应用
 * 数据目录，所以只有第一次是慢的。
 */
export function SchematicMaterialSection() {
  const [candidates, setCandidates] = useState<AssetPackCandidate[] | null>(null);
  const [value, setValue] = useState<string>(loadMaterialPreference);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    let cancelled = false;
    void listAssetPackCandidates()
      .then((list) => {
        if (!cancelled) {
          setCandidates(list);
        }
      })
      .catch((cause: unknown) => {
        if (!cancelled) {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => refresh(), [refresh]);

  const choose = (next: string) => {
    setValue(next);
    saveMaterialPreference(next);
  };

  return (
    <section
      className="status-panel settings-panel"
      aria-labelledby="schematic-material-heading"
    >
      <Text id="schematic-material-heading" as="h2" size={400} weight="semibold">
        投影图材质
      </Text>
      <Text className="muted">
        3D 预览默认用纯色方块；选一个版本后会用它的真实贴图和方块模型。
        第一次选择时要从 jar 里提取，需要等几秒，之后直接读缓存。
      </Text>
      {error ? (
        <Text size={200}>读不到本机版本：{error}</Text>
      ) : candidates === null ? (
        <Spinner size="tiny" />
      ) : (
        <Dropdown
          aria-label="投影图材质来源"
          value={value || "纯色预览"}
          selectedOptions={[value]}
          onOptionSelect={(_, data) => choose(data.optionValue ?? "")}
          className="asset-pack-select"
        >
          <Option value="" text="纯色预览">
            纯色预览（不用贴图）
          </Option>
          {candidates.map((candidate) => (
            <Option
              key={candidate.version}
              value={candidate.version}
              text={candidate.version}
            >
              {candidate.version} · {candidate.sizeMb} MB
              {candidate.extracted ? " · 已提取" : ""}
            </Option>
          ))}
        </Dropdown>
      )}
    </section>
  );
}
