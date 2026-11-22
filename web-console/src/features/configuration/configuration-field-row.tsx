import { Field, Input, Select } from "../../components/ui/field.js";
import { Button } from "../../components/ui/button.js";
import type { ConfigFieldSnapshot } from "../../types.js";
import { configFieldLabel } from "./configuration-navigation.js";
import { configInputValue } from "./configuration-values.js";
import { TTS_PROVIDER_KEY, ttsNumberRange, ttsProviderOptions } from "./tts-config.js";

type PublicFieldRowProps = {
  field: ConfigFieldSnapshot;
  value: string | undefined;
  onChange: (value: string) => void;
  onRemove?: () => void;
  busy?: boolean;
};

/** 普通配置字段行：采用桌面设置面板“左标签说明 / 右控件对齐”结构，字段间由外层容器提供 1px 内分割线。 */
export function PublicFieldRow({ field, value, onChange, onRemove, busy = false }: PublicFieldRowProps) {
  const current = value ?? configInputValue(field);
  const dirty = value !== undefined;
  const id = `config-${field.key}`;
  const range = ttsNumberRange(field.key);
  return (
    <Field
      layout="row"
      label={configFieldLabel(field.key)}
      id={id}
      hint={[
        field.applyMode === "restart" ? "重启后生效" : null,
        field.editable ? null : "只读",
        dirty ? "有未保存修改" : null,
        range ? `范围 ${range[0]} 到 ${range[1]} 的整数` : null,
      ].filter(Boolean).join(" · ") || undefined}
      aside={
        field.savedValue !== null && field.savedValue !== undefined && field.editable && onRemove ? (
          <div>
            <Button variant="ghost" size="sm" disabled={busy} onClick={onRemove}>
              恢复未保存值
            </Button>
          </div>
        ) : null
      }
    >
      {(props) =>
        field.key === TTS_PROVIDER_KEY ? (
          // TTS Provider 是受控下拉：保留未知历史值，避免把自定义 Provider 静默改写。
          <Select
            {...props}
            disabled={!field.editable}
            value={current === "" ? "disabled" : current}
            onChange={(event) => onChange(event.target.value)}
          >
            {ttsProviderOptions(field.savedValue ?? field.effectiveValue).map(([optionValue, label]) => (
              <option key={optionValue} value={optionValue}>{label}</option>
            ))}
          </Select>
        ) : field.valueType === "boolean" ? (
          <Select
            {...props}
            disabled={!field.editable}
            value={current === "true" ? "true" : "false"}
            onChange={(event) => onChange(event.target.value)}
          >
            <option value="true">启用</option>
            <option value="false">关闭</option>
          </Select>
        ) : (
          <Input
            {...props}
            type={field.valueType === "integer" ? "number" : "text"}
            disabled={!field.editable}
            value={current}
            onChange={(event) => onChange(event.target.value)}
          />
        )
      }
    </Field>
  );
}
