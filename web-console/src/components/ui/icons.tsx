import type { SVGProps } from "react";

export type ConsoleIconName =
  | "overview"
  | "platforms"
  | "configuration"
  | "storage"
  | "memory"
  | "todo"
  | "knowledge"
  | "tools"
  | "sun"
  | "moon"
  | "logout"
  | "menu"
  | "close"
  | "refresh"
  | "restart";

const ICON_PATHS: Readonly<Record<ConsoleIconName, readonly string[]>> = {
  overview: ["M4 13h6V4H4v9Z", "M14 20h6v-9h-6v9Z", "M14 4h6v3h-6V4Z", "M4 20h6v-3H4v3Z"],
  platforms: ["M5 5h14v14H5z", "M9 5v14", "M15 5v14", "M5 10h4", "M15 14h4"],
  configuration: [
    "M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7Z",
    "M12 2v3",
    "M12 19v3",
    "M2 12h3",
    "M19 12h3",
    "m4.93 4.93 2.12-2.12",
    "m16.95 7.05 2.12-2.12",
    "m4.93 7.07 2.12 2.12",
    "m16.95 16.95 2.12 2.12",
  ],
  storage: [
    "M4 6.5C4 5.12 7.58 4 12 4s8 1.12 8 2.5S16.42 9 12 9 4 7.88 4 6.5Z",
    "M4 6.5v5C4 12.88 7.58 14 12 14s8-1.12 8-2.5v-5",
    "M4 11.5v6C4 18.88 7.58 20 12 20s8-1.12 8-2.5v-6",
  ],
  memory: ["M5 5h14v14H5z", "M8 9h8", "M8 12h8", "M8 15h5", "M3 9h2", "M19 9h2", "M3 15h2", "M19 15h2"],
  todo: ["M5 6h14", "M5 12h14", "M5 18h9", "M3 6h.01", "M3 12h.01", "M3 18h.01"],
  knowledge: ["M4 5a2 2 0 0 1 2-2h9l5 5v11a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V5Z", "M14 3v5h5", "M8 12h8", "M8 16h5"],
  tools: ["m14.7 6.3 3-3 3 3-3 3", "m17.7 3.3-7.1 7.1", "M5 20h4l8.7-8.7-4-4L5 16v4Z"],
  sun: [
    "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z",
    "M12 2v2",
    "M12 20v2",
    "m4.93 4.93 1.41 1.41",
    "m17.66 17.66 1.41 1.41",
    "M2 12h2",
    "M20 12h2",
    "m4.93 19.07 1.41-1.41",
    "m17.66 6.34 1.41-1.41",
  ],
  moon: ["M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79Z"],
  logout: ["M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4", "m16 17 5-5-5-5", "M21 12H9"],
  menu: ["M4 7h16", "M4 12h16", "M4 17h16"],
  close: ["M6 6l12 12", "M18 6 6 18"],
  refresh: ["M21 12a9 9 0 1 1-2.64-6.36", "M21 3v6h-6"],
  restart: ["M12 2v6", "M6.38 5.29a8 8 0 1 0 11.24 0"],
};

/** 24×24 线性图标：stroke 跟随 currentColor，仅作装饰不承载信息。 */
export function ConsoleIcon({ name, ...props }: { name: ConsoleIconName } & SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 24 24"
      aria-hidden="true"
      focusable="false"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.65}
      strokeLinecap="round"
      strokeLinejoin="round"
      {...props}
    >
      {ICON_PATHS[name].map((d) => (
        <path key={d} d={d} />
      ))}
    </svg>
  );
}
