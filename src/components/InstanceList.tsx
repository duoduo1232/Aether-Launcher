import { Badge, Button, Text, Tooltip } from "@fluentui/react-components";
import { Delete24Regular, Warning24Regular } from "@fluentui/react-icons";
import type { InstanceSummary } from "../ipc";
import { describeInstanceState } from "../lib/instances";
import { formatMemoryMiB } from "../lib/memory";

type InstanceListProps = {
  instances: readonly InstanceSummary[];
  /** 默认内存上限，来自设置；实例行上用同一份值做提示 */
  memoryMiB: number;
  /** 默认 Java 的可读标签，例如 `Java 21 · x64`；没有检测到时为 null */
  javaLabel: string | null;
  /** 删除一个实例记录；不传时行上不显示删除按钮 */
  onDelete?: (instance: InstanceSummary) => void;
  /** 正在删除中的实例 ID，按钮在此期间禁用 */
  deletingId?: string | null;
};

/**
 * 实例列表：只负责展示、搜索过滤后的选中列表，不承载生命周期操作——
 * 启动/停止的唯一入口是列表下方的 `InstanceLaunchPanel`。
 *
 * 删除按钮放在行尾：删的是 Aether 的实例记录，不碰 `.minecraft` 里的游戏文件，
 * 所以不需要额外警告，但用户仍会被问一次（确认交给调用方）。
 *
 * 用 `<ul>` 而不是表格：一行里既有徽章又有次要信息，语义上是「卡片列表」，
 * 窄窗口下也只需要改 `grid-template-columns`。
 */
export function InstanceList({
  instances,
  memoryMiB,
  javaLabel,
  onDelete,
  deletingId = null,
}: InstanceListProps) {
  return (
    <ul className="instance-list">
      {instances.map((instance) => {
        const status = describeInstanceState(instance.state);
        const missingGameDirectory = instance.gameDirectory === null;

        return (
          <li className="instance-row" key={instance.id}>
            <span className="instance-badge" aria-hidden="true">
              {instance.version.slice(0, 6)}
            </span>

            <div className="instance-primary">
              <Text weight="semibold" className="instance-name">
                {instance.name}
              </Text>
              <Text className="muted instance-meta">
                {instance.version}
                <span className="meta-separator" aria-hidden="true">
                  ·
                </span>
                {javaLabel ?? "Java 未检测"}
                <span className="meta-separator" aria-hidden="true">
                  ·
                </span>
                {formatMemoryMiB(memoryMiB)}
              </Text>
              {missingGameDirectory ? (
                <Text className="muted instance-warning">
                  <Warning24Regular /> 还没设置游戏目录，投影和启动都用不了
                </Text>
              ) : null}
            </div>

            <Tooltip content={status.hint} relationship="description">
              <Badge appearance="tint" color={status.tone} className="status-chip">
                {status.label}
              </Badge>
            </Tooltip>

            {onDelete ? (
              <Button
                appearance="subtle"
                size="small"
                icon={<Delete24Regular />}
                aria-label={`删除实例 ${instance.name}`}
                disabled={deletingId === instance.id}
                onClick={() => onDelete(instance)}
              >
                删除
              </Button>
            ) : null}
          </li>
        );
      })}
    </ul>
  );
}
