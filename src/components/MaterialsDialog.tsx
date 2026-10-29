import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Input,
  Text,
} from "@fluentui/react-components";
import { ArrowDownload24Regular, Search20Regular } from "@fluentui/react-icons";
import { useMemo, useState } from "react";
import type { SchematicDetail } from "../ipc";
import {
  describeBoxes,
  materialsToCsv,
  summariseMaterials,
  type MaterialRow,
} from "../lib/materials";

/** 一个潜影盒的槽位数。 */
const SHULKER_SLOTS = 27;

type MaterialsDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  detail: SchematicDetail;
};

/**
 * 材料清单。列出这个投影要用的每种方块、总数，以及换算成
 * 「潜影盒 + 组 + 个」的结果，方便备料。
 *
 * 导出走 CSV：Excel 能直接打开，不需要给前端加 xlsx 依赖。
 */
export function MaterialsDialog({ open, onOpenChange, detail }: MaterialsDialogProps) {
  const [query, setQuery] = useState("");

  const rows = useMemo(() => summariseMaterials(detail), [detail]);
  const visible = useMemo(() => {
    const keyword = query.trim().toLowerCase();
    if (!keyword) {
      return rows;
    }
    return rows.filter((row) => row.label.toLowerCase().includes(keyword));
  }, [rows, query]);

  const totalBlocks = useMemo(() => rows.reduce((sum, row) => sum + row.count, 0), [rows]);

  // 盒子数按**槽位**算，不是按「每种材料各占一盒」：
  //
  // 一个潜影盒 27 格，不同物品放在不同格子里，所以 3 个矮草和 5 个活板门
  // 只占 2 格，能装进同一个盒子。每种材料各自需要的槽位数是
  // `ceil(数量 / 该物品堆叠上限)`，加起来除以 27 才是盒数。
  const totalShulkers = useMemo(() => {
    const slots = rows.reduce((sum, row) => sum + Math.ceil(row.count / row.stackSize), 0);
    return Math.ceil(slots / SHULKER_SLOTS);
  }, [rows]);

  const exportCsv = () => {
    const csv = materialsToCsv(detail.fileName, rows);
    // Excel 默认按本地编码打开 CSV，加 BOM 才不会把中文读成乱码。
    const blob = new Blob([`\uFEFF${csv}`], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `${detail.fileName.replace(/\.litematic$/i, "")}-材料清单.csv`;
    anchor.click();
    URL.revokeObjectURL(url);
  };

  return (
    <Dialog open={open} onOpenChange={(_, data) => onOpenChange(data.open)}>
      <DialogSurface className="materials-surface" aria-describedby={undefined}>
        <DialogBody>
          <DialogTitle>材料清单</DialogTitle>
          <DialogContent className="materials-content">
            <div className="materials-toolbar">
              <Input
                className="materials-search"
                contentBefore={<Search20Regular />}
                placeholder="筛选材料"
                aria-label="筛选材料"
                value={query}
                onChange={(_, data) => setQuery(data.value)}
              />
              <Text className="muted">
                共 {rows.length} 种、{totalBlocks} 个方块、约 {totalShulkers} 盒潜影盒
              </Text>
            </div>

            <div className="materials-table-wrap">
              <table className="materials-table">
                <thead>
                  <tr>
                    <th scope="col">材料</th>
                    <th scope="col">总数</th>
                    <th scope="col">一组</th>
                    <th scope="col">潜影盒</th>
                    <th scope="col">组</th>
                    <th scope="col">个</th>
                    <th scope="col">换算</th>
                  </tr>
                </thead>
                <tbody>
                  {visible.map((row: MaterialRow) => (
                    <tr key={row.block}>
                      <td>{row.label}</td>
                      <td>{row.count}</td>
                      <td>{row.stackSize}</td>
                      <td>{row.shulkers || ""}</td>
                      <td>{row.stacks || ""}</td>
                      <td>{row.loose || ""}</td>
                      <td className="muted">{describeBoxes(row)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {visible.length === 0 ? (
                <Text className="muted materials-empty">
                  没有名称包含「{query.trim()}」的材料。
                </Text>
              ) : null}
            </div>
          </DialogContent>
          <DialogActions>
            <Button icon={<ArrowDownload24Regular />} onClick={exportCsv}>
              导出 Excel
            </Button>
            <Button appearance="primary" onClick={() => onOpenChange(false)}>
              关闭
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
