# Web Console Design System（Atelier Glass · 双栖琉璃）

本文件是生产 Web Console 的视觉与交互契约。

## 1. 核心原则

1. **外透内实的空间层级（Translucent Shell, Crisp Core）**：导航外壳与环境框体采用低不透明度的通透磨砂玻璃，让全局环境画布（或自定义背景）柔和透出；核心读写工作区、表格与表单保持高对比度的纸面/深石板质感。
2. **衬线体作为视觉锚点（Editorial Serif Anchors）**：页面主标题（`h1`）、区块标题（`h2`）与核心实时数值采用衬线体；正文、表单控件与密集列表采用无衬线体；技术数据与快捷键采用等宽字体。
3. **锐利发丝线与柔和圆角（Hairlines & Soft Radius）**：采用 `8px / 12px / 16px` 柔和圆角，依靠 `1px` 半透明发丝线与材质模糊度差异划分边界。
4. **Power User 零阻塞交互（Instant & Quiet Motion）**：取消全屏遮蔽过渡动画，页面切换采用 `120ms` 内容区淡入，微交互仅做克制的色值与边框过渡。

## 2. 空间材质层级

| 层级 | 名称 | 浅色态（晨雾） | 深色态（暮色） | 滤镜 | 用途 |
|---|---|---|---|---|---|
| **L0** | 环境画布（Ambient） | 暖杏 → 雾蓝柔光渐变（`#E6DFD4` → `#CDD7E2`） | 深海石板底叠加暖琥珀与冷月光斑（`#0F1521`） | — | 全屏底层（自定义背景或特殊背景覆盖此层） |
| **L1** | 高通透玻璃外壳（`console-shell`） | `rgb(255 255 255 / 34%)` | `rgb(16 22 34 / 36%)` | `blur(10px) saturate(135%)` | 左侧边栏、顶部状态条、设置二级目录、悬浮保存条、Toast |
| **L2** | 通透雾面面板（`console-panel`） | `rgb(255 255 255 / 44%)` | `rgb(22 30 46 / 42%)` | `blur(8px) saturate(125%)` | 独立区块容器、筛选栏、检查单、认证卡片 |
| **L3** | 通透内容面（`console-paper`） | `rgb(255 255 255 / 56%)` | `rgb(28 38 56 / 52%)` | `blur(6px) saturate(120%)` | 阅读区、表单卡片、表格（输入框 `72% / 64%`，模态框 `84%`） |

## 3. 色彩与排版 Token

语义色定义在 `src/styles/global.css` 的 `:root[data-theme-mode="light"]` 与 `:root[data-theme-mode="dark"]`，预设切换与持久化契约见 [`../docs/THEME.md`](../docs/THEME.md)。

- **衬线标题与关键数值**：`"Source Serif 4", "Charter", "Iowan Old Style", "Noto Serif SC", "Source Han Serif SC", "STZhongsong", Georgia, serif`（标题与 KPI 字重 `600`）
- **正文与控件**：`"IBM Plex Sans", -apple-system, BlinkMacSystemFont, "PingFang SC", "Microsoft YaHei", sans-serif`
- **等宽数据**：`"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace`
- **圆角**：`--console-radius-sm: 8px`（按钮、输入、徽章、胶囊）、`--console-radius: 12px`（卡片、表单区块、表格）、`--console-radius-lg: 16px`（外层玻璃面板、对话框、浮层）。

## 4. 布局与信息架构

- **桌面端（`>= 1024px`）**：固定左侧 `220px` L1 通透玻璃侧边栏，分为「系统运维（总览 / 平台 / 配置 / 存储）」与「核心能力（Memory / Todo / 知识库 / 工具）」，底部常驻昼夜切换与退出登录；右侧顶部为 `48px` L1 状态操作条。
- **平板端（`768px–1023px`）**：侧边栏折叠为 `60px` 图标轨。
- **移动端（`< 768px`）**：左上角菜单按钮唤起玻璃抽屉。
- **杂志感页眉（`PageHeader`）**：左侧大号衬线标题 + 简洁中文副标题；右侧 1–2 个大号衬线核心指标 + 等宽单位。
- **配置中心分栏（`/configuration`）**：左侧 `196px` L1 粘性二级目录（`2px` 强调色竖线标示当前项）+ 右侧 `max-w-[760px]` 左标签说明 / 右控件对齐单列表单 + 仅在有未保存草稿时滑出的底部 L1 确认条。

## 5. 质量底线

- 不在标题上方添加全大写拉宽字距的装饰性英文眉题。
- 状态指示必须同时包含文本标签、几何符号（■ / ◆ / ● / ▲）与语义色。
- 数据表格不使用斑马纹，统一用 `1px` 发丝线与 hover 微弱底色区分行，启用 `tabular-nums`。
- 遵循 `@media (prefers-reduced-motion: reduce)`，关闭非必要过渡动画。
- 所有安全与持久化边界（管理员 Session、同源、CSRF、secret 不回传原文、Markdown 后端 ammonia 清理）保持不变。
