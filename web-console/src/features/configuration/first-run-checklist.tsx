import { useState } from "react";
import { Button } from "../../components/ui/button.js";
import { Frame } from "../../components/ui/frame.js";
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
      "在“模型与供应商”中为至少一个模型服务填入 API Key；该服务必须出现在每条文本模型路线的候选中。停用被路由引用的供应商时，页面会给出告警并在运行时跳过该候选。",
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
      "修改后在底部悬浮条点击“保存普通配置”或“保存密钥变更”，再点页眉“校验配置”。首次安装尚未完成时允许分步保存，未补齐的项会继续显示“需要完成配置”。",
  },
  {
    title: "重启并确认",
    description:
      "点“重启服务”，重新登录后确认总览为“运行正常”、本页为“本地预检通过”，且没有待重启变更。",
  },
];

/**
 * 首次配置检查单：放置在配置中心左侧粘性栏中随页面滚动跟随，
 * 既可随时对照步骤或一键跳转业务域，也不遮挡右侧表单工作区。
 */
export function FirstRunChecklist({
  invalidCount,
  onSelectGroup,
  embedded = false,
}: {
  invalidCount: number;
  onSelectGroup: (group: ConfigurationBusinessGroup) => void;
  /** 嵌入配置中心通高侧栏底部时去掉外层重复框体，与导航目录融为一体。 */
  embedded?: boolean;
}) {
  // 未通过预检时默认展开；通过后保留为可展开的参考清单。
  const [open, setOpen] = useState(invalidCount > 0);
  const body = (
    <>
      <div className="flex items-center justify-between gap-2">
        <h3 className="m-0 text-[13px] font-semibold whitespace-nowrap text-ink">首次配置检查单</h3>
        <Button
          variant="ghost"
          size="sm"
          aria-expanded={open}
          onClick={() => setOpen((current) => !current)}
          className="h-6 shrink-0 px-2 text-[11px]"
        >
          {open ? "收起检查单" : "展开检查单"}
        </Button>
      </div>
      <p className="m-0 mt-1 text-xs leading-relaxed text-muted">
        {invalidCount > 0
          ? "按顺序补齐以下必备项后，本地预检才会通过。"
          : "当前已通过本地预检；仍可展开核对部署所需配置。"}
      </p>
      {open ? (
        <>
          <ol className="m-0 mt-3 flex list-none flex-col divide-y divide-line-inner border-t border-line-inner p-0">
            {CHECKLIST_STEPS.map((step, index) => (
              <li key={step.title} className="flex flex-col gap-1.5 py-2.5 last:pb-0">
                <div className="flex items-center gap-2">
                  <span className="flex size-5 shrink-0 items-center justify-center rounded-console-sm border border-line bg-card font-mono text-[11px] font-semibold text-ink">
                    {index + 1}
                  </span>
                  <p className="m-0 text-xs font-semibold text-ink">{step.title}</p>
                </div>
                <p className="m-0 pl-7 text-[11px] leading-relaxed text-muted">{step.description}</p>
                {step.group && step.groupLabel ? (
                  <div className="pl-7">
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => step.group && onSelectGroup(step.group)}
                      className="h-6 px-2 text-[11px]"
                    >
                      去{step.groupLabel}
                    </Button>
                  </div>
                ) : null}
              </li>
            ))}
          </ol>
          <p className="m-0 mt-2.5 border-t border-line-inner pt-2.5 text-[11px] leading-relaxed text-muted">
            排查指引见 <span className="font-mono text-ink">docs/guides/console-first-run-checklist.md</span>
          </p>
        </>
      ) : null}
    </>
  );

  if (embedded) {
    return (
      <section aria-label="首次配置检查单" className="border-t border-line p-3.5">
        {body}
      </section>
    );
  }

  return (
    <Frame variant="panel" aria-label="首次配置检查单" className="p-3.5">
      {body}
    </Frame>
  );
}
