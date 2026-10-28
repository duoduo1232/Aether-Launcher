import { Button, Dropdown, Option, Spinner, Text } from "@fluentui/react-components";
import { useCallback, useEffect, useState } from "react";
import type { AssetPack, AssetPackCandidate } from "../ipc";
import { listAssetPackCandidates, loadAssetPack } from "../ipc";

/**
 * 「用真实材质显示」的提示条。
 *
 * 为什么不让后端自己解：一个客户端 jar 有二十多 MB，里面上千个 model 和贴图。
 * 打开投影页就自动解压会让每次进页面都等好几秒，所以改成用户明确点一下才做，
 * 提取结果按 jar（路径 + 大小 + 修改时间）缓存，第二次进页面是即时的。
 *
 * 提取完成后把资源包交给上层，预览从纯色方块换成带 UV 的真实模型。
 */
export function AssetPackPicker({
  pack,
  onLoaded,
}: {
  pack: AssetPack | null;
  onLoaded: (pack: AssetPack) => void;
}) {
  const [candidates, setCandidates] = useState<AssetPackCandidate[] | null>(null);
  const [selected, setSelected] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  const extract = useCallback(async () => {
    if (!selected) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      onLoaded(await loadAssetPack(selected));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }, [onLoaded, selected]);

  if (pack) {
    return null;
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
        <Button size="small" onClick={() => void extract()} disabled={busy}>
          重试
        </Button>
      </div>
    );
  }

  return (
    <div className="asset-pack-bar">
      <Text size={200}>当前是纯色预览。</Text>
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
        onClick={() => void extract()}
        disabled={busy || !selected}
      >
        {busy ? "提取中…" : "提取材质"}
      </Button>
    </div>
  );
}
