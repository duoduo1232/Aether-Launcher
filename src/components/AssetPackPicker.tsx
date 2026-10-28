import { Button, Dropdown, Option, Spinner, Text } from "@fluentui/react-components";
import { useCallback, useEffect, useState } from "react";
import type { AssetPack, AssetPackCandidate } from "../ipc";
import { listAssetPackCandidates, loadAssetPack } from "../ipc";
import {
  loadMaterialPreference,
  saveMaterialPreference,
} from "../lib/schematicMaterial";

/**
 * 材质来源选择。
 *
 * 两种形态：
 * - **已经设置过**：什么都不显示。资源包由上层按保存的版本自动加载，用户
 *   进预览直接看到真实材质。
 * - **还没设置**：一条提示 + 版本下拉框 + 「使用材质」。选完立刻保存，之后
 *   不再问 —— 提取结果有后端缓存（按 jar 的路径 + 大小 + 修改时间），再次
 *   打开只是读缓存。
 *
 * 想换版本或改回纯色，去「设置 → 投影图材质」。
 */
export function AssetPackPicker({
  pack,
  onLoaded,
  onPreferenceChange,
}: {
  pack: AssetPack | null;
  onLoaded: (pack: AssetPack) => void;
  /** 保存选择；传空串表示「纯色预览」。 */
  onPreferenceChange: (version: string) => void;
}) {
  const [candidates, setCandidates] = useState<AssetPackCandidate[] | null>(null);
  const [selected, setSelected] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string>(loadMaterialPreference);

  const refresh = useCallback(() => {
    let cancelled = false;
    void listAssetPackCandidates()
      .then((list) => {
        if (cancelled) {
          return;
        }
        setCandidates(list);
        // 默认选已经提取过的那个版本，没有就选最靠前的。
        const ready = list.find((candidate) => candidate.extracted) ?? list[0];
        setSelected((current) => current || ready?.version || "");
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

  const apply = useCallback(
    async (version: string) => {
      if (!version) {
        return;
      }
      setBusy(true);
      setError(null);
      try {
        const pack = await loadAssetPack(version);
        saveMaterialPreference(version);
        setSaved(version);
        onPreferenceChange(version);
        onLoaded(pack);
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        setBusy(false);
      }
    },
    [onLoaded, onPreferenceChange],
  );

  // 已经设置过（且有资源包）：不再打扰。
  if (saved && pack) {
    return null;
  }

  // 设置过但加载失败或正在加载：给一行状态，不再弹选择框。
  if (saved && !pack) {
    return (
      <div className="asset-pack-bar">
        <Spinner size="tiny" />
        <Text size={200}>正在套用材质（MC {saved}）…</Text>
      </div>
    );
  }

  if (candidates === null) {
    return (
      <div className="asset-pack-bar">
        <Spinner size="tiny" />
        <Text size={200}>正在查找本机版本…</Text>
      </div>
    );
  }

  if (candidates.length === 0) {
    return (
      <div className="asset-pack-bar">
        <Text size={200}>
          没有找到客户端 jar。装好一个带客户端的版本后，这里就能提取材质。
        </Text>
      </div>
    );
  }

  if (error) {
    return (
      <div className="asset-pack-bar" role="alert">
        <Text size={200}>提取失败：{error}</Text>
        <Button size="small" onClick={() => void apply(selected)} disabled={busy}>
          重试
        </Button>
      </div>
    );
  }

  return (
    <div className="asset-pack-bar">
      <Text size={200}>
        当前是纯色预览。选一个版本提取真实材质，选完会记住。
      </Text>
      <Dropdown
        size="small"
        aria-label="选择用来提取材质的版本"
        value={selected}
        selectedOptions={selected ? [selected] : []}
        onOptionSelect={(_, data) => setSelected(data.optionValue ?? "")}
        className="asset-pack-select"
      >
        {candidates.map((candidate) => (
          <Option key={candidate.version} value={candidate.version} text={candidate.version}>
            {candidate.version}
            {" · "}
            {candidate.sizeMb} MB
            {candidate.extracted ? " · 已提取" : ""}
          </Option>
        ))}
      </Dropdown>
      <Button
        size="small"
        appearance="primary"
        onClick={() => void apply(selected)}
        disabled={busy || !selected}
      >
        {busy ? "提取中…" : "使用材质"}
      </Button>
    </div>
  );
}
