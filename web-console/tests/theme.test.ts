import { describe, expect, it } from "vitest";
import {
  CONSOLE_THEME_STORAGE_KEY,
  CONSOLE_THEME_VERSION,
  createThemeController,
  defaultThemePreference,
  parseStoredTheme,
  resolveThemeMode,
} from "../src/theme.js";

function createMemoryStorage(): Storage {
  const map = new Map<string, string>();
  return {
    get length() { return map.size; },
    clear: () => map.clear(),
    getItem: (key) => map.get(key) ?? null,
    key: (index) => [...map.keys()][index] ?? null,
    removeItem: (key) => { map.delete(key); },
    setItem: (key, value) => { map.set(key, value); },
  };
}

function createFakeMedia(initialMatches: boolean) {
  let matches = initialMatches;
  const listeners = new Set<() => void>();
  const mql = {
    get matches() { return matches; },
    media: "(prefers-color-scheme: dark)",
    onchange: null,
    addEventListener: (_: string, listener: () => void) => { listeners.add(listener); },
    removeEventListener: (_: string, listener: () => void) => { listeners.delete(listener); },
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false,
  } as unknown as MediaQueryList;
  return {
    mql,
    setMatches(next: boolean) {
      matches = next;
      for (const listener of listeners) listener();
    },
  };
}

describe("Atelier Glass 昼夜主题契约", () => {
  it("默认回退为 system，并按 prefers-color-scheme 解析昼夜模式", () => {
    expect(parseStoredTheme(null)).toEqual(defaultThemePreference());
    expect(resolveThemeMode("system", false)).toBe("light");
    expect(resolveThemeMode("system", true)).toBe("dark");
    expect(resolveThemeMode("mist", true)).toBe("light");
    expect(resolveThemeMode("twilight", false)).toBe("dark");
  });

  it("兼容迁移 v1/v2 旧预设 ID 到晨雾或暮色", () => {
    expect(parseStoredTheme(JSON.stringify({ preset: "console-dark", version: 2 }))).toEqual({
      preset: "twilight",
      version: CONSOLE_THEME_VERSION,
    });
    expect(parseStoredTheme(JSON.stringify({ preset: "night-green", version: 2 }))).toEqual({
      preset: "twilight",
      version: CONSOLE_THEME_VERSION,
    });
    expect(parseStoredTheme(JSON.stringify({ preset: "light", version: 2 }))).toEqual({
      preset: "mist",
      version: CONSOLE_THEME_VERSION,
    });
    expect(parseStoredTheme(JSON.stringify({ preset: "night-shift", version: 1 }))).toEqual({
      preset: "twilight",
      version: CONSOLE_THEME_VERSION,
    });
  });

  it("跟随系统时响应系统昼夜切换，手动选择后锁定昼夜态", () => {
    const storage = createMemoryStorage();
    const root = document.createElement("html");
    const media = createFakeMedia(false);
    const modes: string[] = [];

    const controller = createThemeController(storage, root, media.mql);
    controller.subscribe((mode) => modes.push(mode));

    expect(root.dataset.theme).toBe("system");
    expect(root.dataset.themeMode).toBe("light");

    media.setMatches(true);
    expect(root.dataset.themeMode).toBe("dark");
    expect(modes).toEqual(["dark"]);

    controller.select("mist");
    expect(root.dataset.theme).toBe("mist");
    expect(root.dataset.themeMode).toBe("light");
    expect(storage.getItem(CONSOLE_THEME_STORAGE_KEY)).toBe(
      JSON.stringify({ preset: "mist", version: CONSOLE_THEME_VERSION }),
    );

    media.setMatches(false);
    expect(root.dataset.themeMode).toBe("light");

    controller.reset();
    expect(root.dataset.theme).toBe("system");
    expect(storage.getItem(CONSOLE_THEME_STORAGE_KEY)).toBeNull();
  });

  it("三色自定义覆盖仅写入背景、主文字与强调色，重置后清理", () => {
    const storage = createMemoryStorage();
    const root = document.createElement("html");
    const controller = createThemeController(storage, root, createFakeMedia(false).mql);

    controller.applyCustomColors(["#112233", "#f0f0f0", "#3a4c5e"]);
    expect(root.style.getPropertyValue("--console-background")).toBe("#112233");
    expect(root.style.getPropertyValue("--console-text-primary")).toBe("#F0F0F0");
    expect(root.style.getPropertyValue("--console-accent")).toBe("#3A4C5E");

    controller.select("twilight");
    expect(root.style.getPropertyValue("--console-background")).toBe("");
  });
});
