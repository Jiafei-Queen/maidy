export const CONSOLE_THEME_STORAGE_KEY = "console-theme";
export const CONSOLE_THEME_VERSION = 3;
export const DEFAULT_CONSOLE_THEME = "system";

/**
 * Atelier Glass 昼夜双态：mist（晨雾与纸）/ twilight（暮色窗棂），
 * system 跟随操作系统的 prefers-color-scheme 自动切换。
 */
export const CONSOLE_THEME_IDS = ["system", "mist", "twilight"] as const;
export type ConsoleThemePreset = (typeof CONSOLE_THEME_IDS)[number];
export type ConsoleThemeMode = "light" | "dark";

export type ConsoleTheme = {
  readonly id: ConsoleThemePreset;
  readonly name: string;
  readonly description: string;
  /** 预览色块：画布、核心内容面、强调色；system 预设展示浅色态。 */
  readonly swatches: readonly [string, string, string];
};

export type ConsoleThemePreference = {
  readonly preset: ConsoleThemePreset;
  readonly version: typeof CONSOLE_THEME_VERSION;
  readonly customColors?: readonly string[];
};

/** 色值本身集中在 global.css 的 [data-theme-mode] 选择器中，这里只保留展示元数据。 */
export const CONSOLE_THEMES = {
  system: {
    id: "system",
    name: "跟随系统",
    description: "按操作系统的浅色 / 深色偏好在晨雾与暮色间切换",
    swatches: ["#E8E2D8", "#1E2738", "#3A4C5E"],
  },
  mist: {
    id: "mist",
    name: "晨雾",
    description: "暖杏雾蓝画布上的纸面工作台",
    swatches: ["#E8E2D8", "#FFFFFF", "#3A4C5E"],
  },
  twilight: {
    id: "twilight",
    name: "暮色",
    description: "深海石板与暖琥珀光斑的夜间工作台",
    swatches: ["#101520", "#1E2738", "#9CA0E6"],
  },
} as const satisfies Readonly<Record<ConsoleThemePreset, ConsoleTheme>>;

/** 旧版预设 ID → 新双态：深色系统一迁移为暮色，日间纸面迁移为晨雾。 */
const LEGACY_THEME_MIGRATIONS: Readonly<Record<string, ConsoleThemePreset>> = {
  "console-dark": "twilight",
  "night-green": "twilight",
  "night-shift": "twilight",
  "ember-grid": "twilight",
  "tide-signal": "twilight",
  light: "mist",
};

/** 三色自定义覆盖的 CSS 变量：画布底色、主文字、强调色。 */
const CUSTOM_COLOR_PROPERTIES = ["--console-background", "--console-text-primary", "--console-accent"] as const;

const DARK_QUERY = "(prefers-color-scheme: dark)";

export function isConsoleThemePreset(value: string): value is ConsoleThemePreset {
  return Object.hasOwn(CONSOLE_THEMES, value);
}

export function parseStoredTheme(value: string | null): ConsoleThemePreference {
  if (value === null) return defaultThemePreference();
  try {
    const parsed: unknown = JSON.parse(value);
    if (!isRecord(parsed) || typeof parsed.preset !== "string") return defaultThemePreference();
    if (parsed.version === CONSOLE_THEME_VERSION && isConsoleThemePreset(parsed.preset)) {
      return { preset: parsed.preset, version: CONSOLE_THEME_VERSION };
    }
    // v1/v2 的旧预设只做读取迁移，不把旧值写回 localStorage。
    if (parsed.version === 1 || parsed.version === 2) {
      const preset = LEGACY_THEME_MIGRATIONS[parsed.preset];
      if (preset !== undefined) return { preset, version: CONSOLE_THEME_VERSION };
    }
    return defaultThemePreference();
  } catch (cause) {
    if (cause instanceof Error) return defaultThemePreference();
    throw cause;
  }
}

export function serializeTheme(preference: ConsoleThemePreference): string {
  return JSON.stringify(preference);
}

export function defaultThemePreference(): ConsoleThemePreference {
  return { preset: DEFAULT_CONSOLE_THEME, version: CONSOLE_THEME_VERSION };
}

export function safeCustomColors(colors: readonly string[]): readonly string[] {
  return colors.filter((color) => /^#[0-9a-f]{6}$/i.test(color)).slice(0, 3).map((color) => color.toUpperCase());
}

/** 把预设解析为实际昼夜模式；system 依赖 matchMedia，不可用时按浅色处理。 */
export function resolveThemeMode(preset: ConsoleThemePreset, prefersDark: boolean): ConsoleThemeMode {
  if (preset === "mist") return "light";
  if (preset === "twilight") return "dark";
  return prefersDark ? "dark" : "light";
}

export type ThemeController = {
  readonly current: () => ConsoleThemePreference;
  /** 当前实际生效的昼夜模式（system 已解析）。 */
  readonly mode: () => ConsoleThemeMode;
  readonly select: (preset: ConsoleThemePreset) => ConsoleThemePreference;
  readonly apply: (preference: ConsoleThemePreference) => void;
  readonly reset: () => ConsoleThemePreference;
  readonly applyCustomColors: (colors: readonly string[]) => ConsoleThemePreference;
  readonly hydrate: (preference: { readonly preset?: string; readonly customColors?: readonly string[] }) => void;
  /** 订阅实际模式变化（含系统偏好切换）；返回取消函数。 */
  readonly subscribe: (listener: (mode: ConsoleThemeMode) => void) => () => void;
};

export function createThemeController(storage: Storage | null, root: HTMLElement, media?: MediaQueryList | null): ThemeController {
  const darkQuery = media === undefined ? safeMatchMedia() : media;
  const listeners = new Set<(mode: ConsoleThemeMode) => void>();
  let current = readStoredTheme(storage);
  let mode = currentMode();
  applyTheme(root, current, mode);

  function currentMode(): ConsoleThemeMode {
    return resolveThemeMode(current.preset, darkQuery?.matches ?? false);
  }

  function commit(next: ConsoleThemePreference): void {
    current = next;
    mode = currentMode();
    applyTheme(root, current, mode);
    for (const listener of listeners) listener(mode);
  }

  // 跟随系统时监听偏好变化；显式选择晨雾 / 暮色时 currentMode 会忽略系统值。
  darkQuery?.addEventListener?.("change", () => {
    if (current.preset === "system") commit(current);
  });

  return {
    current: () => current,
    mode: () => mode,
    apply: (preference) => {
      commit(preference);
      writeStoredTheme(storage, current);
    },
    select: (preset) => {
      commit({ preset, version: CONSOLE_THEME_VERSION });
      writeStoredTheme(storage, current);
      return current;
    },
    reset: () => {
      commit(defaultThemePreference());
      removeStoredTheme(storage);
      return current;
    },
    applyCustomColors: (colors) => {
      const customColors = safeCustomColors(colors);
      commit(customColors.length === 3 ? { ...current, customColors } : { preset: current.preset, version: CONSOLE_THEME_VERSION });
      return current;
    },
    hydrate: (preference) => {
      const preset = typeof preference.preset === "string" && isConsoleThemePreset(preference.preset)
        ? preference.preset
        : DEFAULT_CONSOLE_THEME;
      const customColors = safeCustomColors(preference.customColors ?? []);
      commit(customColors.length === 3 ? { preset, version: CONSOLE_THEME_VERSION, customColors } : { preset, version: CONSOLE_THEME_VERSION });
    },
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

function safeMatchMedia(): MediaQueryList | null {
  try {
    return typeof window !== "undefined" && typeof window.matchMedia === "function" ? window.matchMedia(DARK_QUERY) : null;
  } catch {
    return null;
  }
}

function readStoredTheme(storage: Storage | null): ConsoleThemePreference {
  if (storage === null) return defaultThemePreference();
  try {
    return parseStoredTheme(storage.getItem(CONSOLE_THEME_STORAGE_KEY));
  } catch {
    return defaultThemePreference();
  }
}

function writeStoredTheme(storage: Storage | null, preference: ConsoleThemePreference): void {
  if (storage === null) return;
  try {
    // 自定义色由服务端偏好持有，localStorage 只记录预设 ID。
    storage.setItem(CONSOLE_THEME_STORAGE_KEY, serializeTheme({ preset: preference.preset, version: CONSOLE_THEME_VERSION }));
  } catch {
    return;
  }
}

function removeStoredTheme(storage: Storage | null): void {
  if (storage === null) return;
  try {
    storage.removeItem(CONSOLE_THEME_STORAGE_KEY);
  } catch {
    return;
  }
}

function applyTheme(root: HTMLElement, preference: ConsoleThemePreference, mode: ConsoleThemeMode): void {
  root.dataset.theme = preference.preset;
  root.dataset.themeMode = mode;
  if (!root.style) return;
  for (const property of CUSTOM_COLOR_PROPERTIES) root.style.removeProperty(property);
  const colors = preference.customColors;
  if (colors?.length === 3) {
    // 兼容已有三色用户偏好：依次覆盖画布底色、主文字与强调色，其余材质层级仍由昼夜态提供。
    CUSTOM_COLOR_PROPERTIES.forEach((property, index) => root.style.setProperty(property, colors[index] ?? ""));
  }
}

function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === "object" && value !== null;
}
