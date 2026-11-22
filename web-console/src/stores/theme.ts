import { atom, getDefaultStore } from "jotai";
import {
  DEFAULT_CONSOLE_THEME,
  createThemeController,
  type ConsoleThemeMode,
  type ConsoleThemePreset,
} from "../theme.js";

/**
 * 主题偏好镜像：theme controller 是命令式单例（负责写 root dataset、自定义色与 localStorage），
 * 这两个 atom 只把当前预设与实际昼夜模式暴露给 React 组件做只读订阅（如编辑器配色跟随）。
 */
export const themePresetAtom = atom<ConsoleThemePreset>(DEFAULT_CONSOLE_THEME);
export const themeModeAtom = atom<ConsoleThemeMode>("light");

function safeStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** 主题 controller 单例：模块加载时立即应用已保存主题，避免首帧闪烁。 */
export const themeController = createThemeController(safeStorage(), document.documentElement);

const store = getDefaultStore();
store.set(themePresetAtom, themeController.current().preset);
store.set(themeModeAtom, themeController.mode());
// 系统偏好切换或用户选择后，同步实际模式与预设镜像。
themeController.subscribe((mode) => {
  store.set(themeModeAtom, mode);
  store.set(themePresetAtom, themeController.current().preset);
});

/** 侧边栏昼夜开关：在晨雾与暮色间显式切换（离开“跟随系统”）。 */
export function toggleThemeMode(): void {
  // 保留已同步的服务端自定义色，只切换预设；localStorage 仅记录预设 ID。
  const current = themeController.current();
  themeController.apply({ ...current, preset: themeController.mode() === "dark" ? "mist" : "twilight" });
}
