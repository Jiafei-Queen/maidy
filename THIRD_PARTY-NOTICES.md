# Third-Party Notices

本文件汇总 `maidy` 中包含的第三方代码、数据和素材的版权与许可信息。

---

## 本项目与上游项目的关系

`maidy` 是基于 [`kuliantnt/qq-maid-bot`](https://github.com/kuliantnt/qq-maid-bot)（曾用名「小女仆机器人」）的衍生作品
（derivative work），fork 自上游 commit `7af34fd288f55c7fb3250387dde0db580d9fbba9`（v0.26.1）。

上游项目以 MIT License 发布，Copyright (c) 2026 kuliantnt。原始许可证全文见仓库根目录 `LICENSE`，
本项目对上游代码的修改、重命名与再分发均遵循该许可证条款。

根据 MIT License 的要求，原始版权声明与许可声明已完整保留于 `LICENSE` 中。

---

## xai-grok-memory Dream

本项目以下文件的部分实现改编自
[`xai-org/grok-build`](https://github.com/xai-org/grok-build/tree/8adf9013a0929e5c7f1d4e849492d2387837a28d/crates/codegen/xai-grok-memory)
中的 Dream 实现，包括其阈值/检查点流程、输入截断行为、prompt 规则、`NO_REPLY` 处理、
校验边界以及失败/成功处理语义：

- `qq-maid-core/src/runtime/tools/memory/storage/dream.rs`
- `qq-maid-core/src/runtime/tools/memory/dream/`（目录整体）

Copyright 2023-2026 SpaceXAI.

Licensed under the Apache License, Version 2.0.

改编后的实现已针对本项目的 SQLite Session 存储、服务端持有的 Personal/GroupProfile 目标、
严格 JSON 输出、群成员 opt-out 与事务性 Memory 写入进行了修改，因此该等文件属于
Apache License 2.0 第 4 条意义上的修改文件（modified files），不与本项目其余 MIT 代码混同授权。
Apache License 2.0 全文副本见 `LICENSES/Apache-2.0.txt`。

> 说明：上游 `THIRD_PARTY-NOTICES.md` 曾将该目录记为
> `qq-maid-core/src/runtime/tools/memory/dream.rs`（单文件），但上游实际结构为目录
> `qq-maid-core/src/runtime/tools/memory/dream/`，此处的路径记录以实际代码结构为准。

---

## Models.dev model catalog

以下文件派生自 [`anomalyco/models.dev`](https://github.com/anomalyco/models.dev) / [Models.dev](https://models.dev)：

- `qq-maid-llm/assets/model-catalog.json`
- `qq-maid-llm/assets/models-dev/` 下的固定转换器输入

Copyright (c) 2025 models.dev

Licensed under the MIT License。许可证全文副本见 `LICENSES/MIT.txt`。

内嵌快照是由 `qq-maid-llm/src/model_catalog/converter.rs` 执行的确定性、仅白名单提取，
仅包含模型元数据，不重新分发 Models.dev 的 logo 或无关素材。

---

## 已移除的第三方内容

以下内容在上游仓库中存在，但**未包含在本项目中**，因此不构成本项目的第三方许可义务：

| 内容 | 原因 |
| --- | --- |
| `docs/img/AUV_LOGO.png`、`docs/img/AUV_LOGO.svg` | 第三方服务商 `CodexAuv` 的品牌标识素材，其版权与商标权归该服务商所有，未被上游以 MIT 授权。本项目为独立品牌，不沿用该素材 |
| `README.md` 中的推广/返利链接（`codexauv.com/register?aff=...`） | 指向第三方的带佣金参数的推广链接，属于上游作者的商业合作内容，与本项目无关 |

其余图片素材（`assets/favicon.png`、`assets/special-sprite.webp`、`docs/img/logo.png`、
`docs/img/readme-chat-demo.png`、`docs/img/readme-health-demo.png`）随上游仓库以 MIT License 一并提供。

---

## 素材授权状态（需自行确认）

`docs/img/logo.png`、`assets/favicon.png` 与 `assets/special-sprite.webp` 的人物形象与美术素材，
其著作权状态未在上游仓库中单独声明。本项目沿用这些文件是承继上游 MIT 许可的结果，
但**不对此类美术素材的独立商用授权作任何额外保证**。如果本项目将用于商业用途或公开发布品牌形象，
建议替换为自有素材。
