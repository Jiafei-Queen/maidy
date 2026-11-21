import { useState } from "react";
import { Button } from "../../components/ui/button.js";
import type { ConfigurationBusinessGroup } from "./configuration-navigation.js";

interface ChecklistStep {
  title: string;
  description: string;
  group?: ConfigurationBusinessGroup;
  groupLabel?: string;
}

/** 与启动预检对应的必备项；这里的文案只做引导，最终结果以“本地预检”徽章为准。 */
const CHECKLIST_STEPS: ReadonlyArray<ChecklistStep> = [
  {
    title: "配置模型凭证",
    description:
      "在“模型与供应商”中为至少一个模型服务填入 API Key；该服务必须出现在每条文本模型路线的候选中。停用仍被路线引用的供应商会导致预检失败。",
    group: "models-providers",
    groupLabel: "模型与供应商",
  },
  {
    title: "补全一个平台入口",
    description:
      "在“平台接入”中三选一：QQ 官方需要 AppID 与 AppSecret；OneBot 11 需要启用后填写 Access Token；微信服务号需要启用后填写 Token，AES 模式还需 AppID 与 EncodingAESKey。",
    group: "platforms",
    groupLabel: "平台接入",
  },
  {
    title: "核对文本模型路由",
    description:
      "默认 Agent 模板通常无需修改。改过路线时，确认每条路线至少有一个可用候选，且没有引用已停用模型或未声明的自定义 Provider。",
    group: "model-routing",
    groupLabel: "文本模型路由",
  },
  {
    title: "保存并校验",
    description:
      "使用页面底部的“保存普通配置”和“保存密钥变更”，再点“校验配置”。首次安装尚未完成时允许分步保存，未补齐的项会继续显示“需要完成配置”。",
  },
  {
    title: "重启并确认",
    description:
      "点“重启服务”，重新登录后确认总览为“健康”、本页为“本地预检通过”，且没有待重启变更。",
  },
];

/** 首次配置检查单：把启动预检的必备项按操作顺序列出，并提供业务域跳转。 */
export function FirstRunChecklist({
  invalidCount,
  onSelectGroup,
}: {
  invalidCount: number;
  onSelectGroup: (group: ConfigurationBusinessGroup) => void;
}) {
  // 未通过预检时默认展开；通过后保留为可展开的参考清单。
  const [open, setOpen] = useState(invalidCount > 0);
  return (
    <section aria-label="首次配置检查单" className="mt-4 rounded-console-lg border border-line bg-glass-muted p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="m-0 text-base font-bold">首次配置检查单</h3>
          <p className="m-0 mt-1 text-xs leading-relaxed text-muted">
            {invalidCount > 0
              ? "按顺序补齐以下必备项后，本地预检才会通过。"
              : "当前已通过本地预检；仍可展开核对部署所需配置。"}
          </p>
        </div>
        <Button
          variant="secondary"
          aria-expanded={open}
          onClick={() => setOpen((current) => !current)}
          className="px-2.5 py-1 text-xs"
        >
          {open ? "收起检查单" : "展开检查单"}
        </Button>
      </div>
      {open ? (
        <>
          <ol className="m-0 mt-3 flex list-none flex-col gap-3 p-0">
            {CHECKLIST_STEPS.map((step, index) => (
              <li
                key={step.title}
                className="flex flex-wrap items-start gap-3 border-t border-line-inner pt-3 first:border-t-0 first:pt-0"
              >
                <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-accent-soft font-mono text-xs font-bold text-accent-strong">
                  {index + 1}
                </span>
                <div className="min-w-0 flex-1">
                  <p className="m-0 text-sm font-bold">{step.title}</p>
                  <p className="m-0 mt-1 text-xs leading-relaxed text-muted">{step.description}</p>
                </div>
                {step.group && step.groupLabel ? (
                  <Button
                    variant="secondary"
                    onClick={() => step.group && onSelectGroup(step.group)}
                    className="px-2.5 py-1 text-xs"
                  >
                    去{step.groupLabel}
                  </Button>
                ) : null}
              </li>
            ))}
          </ol>
          <p className="m-0 mt-3 border-t border-line-inner pt-3 text-xs text-muted">
            完整排查步骤见仓库 <span className="font-mono">docs/guides/console-first-run-checklist.md</span>。
          </p>
        </>
      ) : null}
    </section>
  );
}
