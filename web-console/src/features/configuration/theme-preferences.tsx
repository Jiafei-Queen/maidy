import { useState } from "react";
import { useAtomValue } from "jotai";
import { CONSOLE_THEMES, CONSOLE_THEME_IDS, isConsoleThemePreset } from "../../theme.js";
import { themeController, themePresetAtom } from "../../stores/theme.js";
import { updatePreferences } from "../../stores/user-data.js";
import { Button } from "../../components/ui/button.js";
import { Frame } from "../../components/ui/frame.js";
import { Input } from "../../components/ui/field.js";

const COLOR_INPUT_PATTERN = /^#[0-9a-fA-F]{6}$/;

/**
 * 主题偏好：Atelier Glass 双态预设选择（跟随系统 / 晨雾 / 暮色）与三色自定义覆盖。
 * 预设 ID 保存在 localStorage；切换预设会清空服务端自定义色；恢复默认同步重置两者。
 */
export function ThemePreferencesSection() {
  const preset = useAtomValue(themePresetAtom);
  const [status, setStatus] = useState("当前主题：" + CONSOLE_THEMES[preset].name);
  const [customColors, setCustomColors] = useState("");
  const [saveInFlight, setSaveInFlight] = useState(false);

  const savePreference = async (
    patch: Parameters<typeof updatePreferences>[0],
    success: string,
    failure: string,
    apply: () => void,
  ): Promise<void> => {
    if (saveInFlight) return;
    setSaveInFlight(true);
    setStatus("正在保存界面偏好……");
    try {
      await updatePreferences(patch);
      apply();
      setStatus(success);
    } catch (cause) {
      setStatus(cause instanceof Error ? `${failure}：${cause.message}` : failure);
    } finally {
      setSaveInFlight(false);
    }
  };

  const selectPreset = (nextPreset: string): void => {
    if (!isConsoleThemePreset(nextPreset)) return;
    void savePreference({ customColors: [] }, "主题已保存。", "主题保存失败", () => {
      const preference = themeController.select(nextPreset);
      setStatus(`当前主题：${CONSOLE_THEMES[preference.preset].name}`);
    });
  };

  const saveCustomColors = (): void => {
    const colors = customColors.split(",").map((value) => value.trim());
    if (colors.length !== 3 || colors.some((color) => !COLOR_INPUT_PATTERN.test(color))) {
      setStatus("请输入三个六位十六进制颜色。");
      return;
    }
    void savePreference({ customColors: colors }, "自定义颜色已保存。", "自定义颜色保存失败", () => {
      themeController.applyCustomColors(colors);
    });
  };

  return (
    <section aria-label="界面主题偏好" className="flex flex-col gap-2.5">
      <h3 className="m-0 text-sm font-semibold text-ink">外观与主题</h3>
      <Frame variant="card" className="flex flex-col gap-4">
        <div role="radiogroup" aria-label="主题预设" className="grid grid-cols-1 gap-2 sm:grid-cols-3">
          {CONSOLE_THEME_IDS.map((id) => {
            const theme = CONSOLE_THEMES[id];
            const selected = preset === id;
            return (
              <label
                key={id}
                className={`flex cursor-pointer flex-col gap-2 rounded-console-sm border p-3 transition-colors duration-[120ms] ease-out ${
                  selected ? "border-accent bg-accent-soft" : "border-line bg-card/50 hover:bg-accent-soft/50"
                }`}
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <input
                      type="radio"
                      name="console-theme"
                      value={id}
                      checked={selected}
                      onChange={() => selectPreset(id)}
                      className="size-3.5 accent-[var(--console-accent)]"
                    />
                    <span className="text-[13px] font-semibold text-ink">{theme.name}</span>
                  </div>
                  <span aria-label={`${theme.name} 的画布、内容面和强调色预览`} className="flex gap-1">
                    {theme.swatches.map((hex, index) => (
                      <span
                        key={index}
                        title={index === 0 ? "画布" : index === 1 ? "内容面" : "强调色"}
                        style={{ backgroundColor: hex }}
                        className="inline-block size-3 rounded-[4px] border border-line"
                      />
                    ))}
                  </span>
                </div>
                <span className="text-xs leading-relaxed text-muted">{theme.description}</span>
              </label>
            );
          })}
        </div>

        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-line-inner pt-3">
          <p aria-live="polite" role="status" className="m-0 text-xs text-muted">
            {status}
          </p>
          <Button
            variant="ghost"
            size="sm"
            disabled={saveInFlight}
            onClick={() => {
              void savePreference({ customColors: [] }, "已恢复默认主题。", "恢复默认主题失败", () => {
                const preference = themeController.reset();
                setStatus(`当前主题：${CONSOLE_THEMES[preference.preset].name}`);
              });
            }}
          >
            恢复默认
          </Button>
        </div>

        <div className="flex flex-col gap-1.5 border-t border-line-inner pt-3">
          <label htmlFor="custom-colors-input" className="text-[13px] font-medium text-ink">
            自定义颜色（画布、主文字、强调色）
          </label>
          <div className="flex flex-wrap gap-2">
            <Input
              id="custom-colors-input"
              type="text"
              value={customColors}
              placeholder="#E8E2D8, #1C232B, #3A4C5E"
              onChange={(event) => setCustomColors(event.target.value)}
              className="max-w-96 flex-1 font-mono"
            />
            <Button variant="secondary" size="sm" disabled={saveInFlight} onClick={saveCustomColors}>
              保存颜色
            </Button>
          </div>
        </div>
      </Frame>
    </section>
  );
}
