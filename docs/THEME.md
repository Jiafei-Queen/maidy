# Console Theme Contract

## Boundary

主题预设是浏览器界面偏好，不是机器人运行配置。预设 ID 只存在当前浏览器 origin 的 `localStorage`，不写入 `runtime.toml`、SQLite、secret store、session、cookie、CSRF header 或业务 API。

登录后的自定义颜色继续沿用管理员用户偏好 API；它只覆盖画布底色（`--console-background`）、主文字（`--console-text-primary`）和强调色（`--console-accent`），不改变昼夜材质与状态语义。

## Presets（Atelier Glass 双态）

生产控制台提供三套预设，对应浅色态（晨雾与纸）与深色态（暮色窗棂）：

| ID | 名称 | 昼夜模式 | 主文字 | 次文字 | 主强调色 | 温馨暖色 |
|---|---|---|---|---|---|---|
| `system` | 跟随系统（默认） | 跟随 `prefers-color-scheme` | 自动切换 | 自动切换 | 自动切换 | 自动切换 |
| `mist` | 晨雾 | `light` | `#1C232B` | `#5E6873` | `#3A4C5E` | `#B58338` |
| `twilight` | 暮色 | `dark` | `#F2EDE4` | `#8F9BAE` | `#9CA0E6` | `#D4A36A` |

色值集中定义在 `web-console/src/styles/global.css` 的 `:root[data-theme-mode="light"]` 与 `:root[data-theme-mode="dark"]`，组件只能消费 CSS 语义 token。

## Storage Format

```text
key: console-theme
value: {"preset":"system","version":3}
```

规则：

1. 页面加载时读取并解析；缺失、损坏或未知 preset 回退 `system`。
2. 回退不立即写入，只有用户主动选择时才保存。
3. 用户选择新 preset 时同步写入并立即更新 root 的 `data-theme` 与 `data-theme-mode`。
4. “恢复默认”删除该 key，并应用 `system`。
5. v1/v2 旧预设无损迁移：`console-dark`、`night-green`、`night-shift`、`ember-grid`、`tide-signal` 迁移为 `twilight`；`light` 迁移为 `mist`。
6. 不存放用户名、session、CSRF、secret、自由文本或后端响应。
7. 读取 localStorage 失败时只使用内存默认值，不阻塞页面。

## 切换入口

- **侧边栏底部快捷开关**：在「晨雾」与「暮色」之间一键切换。
- **配置中心 → 系统与安全 → 外观与主题**：选择「跟随系统 / 晨雾 / 暮色」、恢复默认或配置三色自定义覆盖。
