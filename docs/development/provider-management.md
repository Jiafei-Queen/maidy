# 供应商管理

对应 [#684 阶段 1 / 2](https://github.com/kuliantnt/qq-maid-bot/issues/684) 及后续供应商与模型管理重构。存储优先级、管理员认证与备份规则沿用[配置中心](./config-center.md)。

## 使用流程

在“模型与供应商”页面，所有供应商采用统一主从列表管理：

- **统一平等供应商（`managed_providers = true`）**：默认模板 `runtime/config/agent.example.toml` 启用 `managed_providers = true`，OpenAI、Gemini、DeepSeek、智谱 BigModel、MiMo 与新建连接一样全部定义在 `agent.toml [providers.*]` 中，均支持编辑、停用、删除，并可通过「＋ 新建供应商」从预设一键重新添加。仅底层协议适配器（`openai_responses`、`openai_compatible`）、Gemini 原生搜索与 DeepSeek 旧模型兼容规则保留为代码级能力，不再锁定不可删除的品牌内置卡片。
- **旧版兼容**：未显式开启 `managed_providers = true` 的历史 `agent.toml` 继续兼容旧内置字段（`provider.<id>.*`）与 `agent.toml` 自定义连接并存。
- **新建与预设**：点击「＋ 新建供应商」选择受信预设或自定义连接，自动填充规范 ID、显示名称、协议与 Base URL。新建 Connection 必须使用小写 canonical ID；历史非小写 key 只能在原始 key 精确匹配时编辑或删除，不能改成 canonical 小写，也不能与仅大小写不同的 key 共存。创建后可新增或替换 API Key，并填写真实模型 ID 执行服务端连接测试。
- **删除供应商**：删除操作支持勾选「同时从受影响的路由中移除候选引用」（`remove_provider` 的 `cleanup_references: true`），在同一原子提交内从文本模型路线与 STT 路线移除对应候选；若移除后某条模型路线会变空，或活跃的 `provider_native` 搜索路线仍引用该供应商，服务端仍会拒绝并明确指出引用位置。

## 软停用与全链路路由健康告警

- 停用任意供应商（包括 OpenAI）不再硬性阻断保存，也不会连带破坏其他已启用供应商；运行时会自动跳过已停用供应商的候选，按候选链顺序回退到后续可用候选。
- 当停用或缺少凭证的供应商仍被路线引用时，系统在三处给出实时告警：
  1. **供应商详情页**：停用时立即列出受影响的模型/搜索/STT 路线，标明是否会导致活跃路线无可用候选；
  2. **模型/搜索/STT 路由编辑器**：每个不可用候选 Chip 展示状态徽标与具体原因（供应商已停用 / 未配置 / 未配 Key / 模型已禁用），整条活跃路线无可用候选时显示红色严重警告，并提供「清理失效项」一键移除按钮；
  3. **配置页顶部摘要**：汇总活跃空路线数量与不可用候选数量。
- 内置或标准 DeepSeek 连接测试按运行时规则选择：旧 `deepseek-chat` / `deepseek-reasoner` 使用 Chat，其他模型使用 Responses，沿用[LLM crate 的使用边界](../../qq-maid-llm/README.md#deepseek-responses-使用边界)。Gemini 在此测试其现有 Chat Adapter，不代表原生搜索已经验证。

## 安全与兼容

- 受信预设在 Core `config/center/provider_presets.rs` 随版本发布，包含 OpenAI（Responses / Chat）、Gemini、DeepSeek、智谱 BigModel、MiMo 以及 OpenCode 模板。前端不维护品牌协议或连接安全配置。
- 新连接的 Credential Slot 由服务端随机生成；浏览器不能指定任意环境变量或 Secret 路径。一个新连接对应一个加密 Slot，API 只返回配置状态和 opaque revision。
- 既有 `api_key_env`（含默认模板复用的 `OPENAI_API_KEY`、`DEEPSEEK_API_KEY` 等历史环境变量）保持兼容；指向已登记 Secret 字段时继续复用原 CAS 接口，其他历史环境变量只读展示。
- 删除连接会归档其专属加密密文并清理对应的模型发现缓存；共享历史凭证不随连接删除。
- API Key 不写入 `agent.toml`、`models.json`、`models-discovery-cache.json`，不回传页面、不进入诊断结果；供应商凭证保存成功后清空输入及已保存明文状态。配置保存继续使用独立 revision、原子写入及 `pending_restart`。
- 没有数据库 schema 变更，也不在启动时改写已有用户配置文件。

## 管理 API

现有 `GET /api/v1/console/configuration` 的 `configuration.providers` 提供受信预设、已注册 Adapter、`managed_providers` 标志、各连接 Credential 状态及 `available_models`（各供应商当前可供路由下拉选择的模型列表）；`configuration.route_diagnostics` 返回文本、搜索与 STT 路线的实时健康诊断。

- `PATCH .../configuration/agent`：复用 `set_provider` / `remove_provider`；`remove_provider` 支持可选布尔字段 `cleanup_references`（默认 `false`）。新建连接时省略 `api_key_env`，既有连接省略时保留原引用。
- `PATCH .../configuration/providers/credential`：`id`、`expected_agent_revision`、`expected_revision`、`value`；字符串替换，`null` 清除。两个 revision 均须匹配。
- `POST .../configuration/providers/test`：`id`、`expected_revision`、`model`；统一/自定义连接使用 Agent revision，旧版内置连接使用 runtime revision。测试的是已保存配置，浏览器不提交目标 URL 或凭证。

测试由 LLM Adapter 层构造最小真实请求，固定文本、无工具、最多 32 个输出 token；可能产生供应商计费。连接超时 5 秒、总超时 15 秒、响应上限 64 KiB、最多两个并发测试，禁止重定向。不回传上游响应正文和原始错误 URL。结果包含真实耗时、HTTP 状态、稳定分类和 `network/authentication/adapter/model_call` 各阶段的 `success/failed/unknown/not_tested`。

最近测试结果仅保留在当前页面会话，连接或凭证 revision 变化后失效；重新登录/初始化后清空，不代替运行时健康状态。

## Connection 模型发现与持久化缓存

`POST /api/v1/console/configuration/providers/models` 接收 `id` 和 `expected_revision`，只读取已保存连接，不写配置或路线。接口仍要求管理员 Session、Origin 与 CSRF；浏览器不能提交 URL、认证或 Credential。响应包含 `connection_id`、请求的 `revision` 和 `discovery`。

模型发现依据 Connection 实际使用的协议 Adapter（`openai_responses` / `openai_compatible`），从连接自身 Base URL 追加 `/models`，沿用 Header、Scheme 和当前解析出的 Credential。端点返回 404 / 405 / 501 时由 discovery 层返回 `unsupported / endpoint_unsupported`，不按品牌预判支持情况。协议参考 [OpenAI 模型列表](https://platform.openai.com/docs/api-reference/models/list)和 [DeepSeek 模型列表](https://api-docs.deepseek.com/api/list-models)。

发现沿用自定义 `request_timeout_seconds`，缺省用 `LLM_REQUEST_TIMEOUT_SECONDS`，再限制总等待不超过 15 秒、连接不超过 5 秒。最多两个并发，忙时立即返回 `unknown / busy`；禁止重定向、自动重试与分页跳转。响应上限 1 MiB、最多 10000 个条目；model id 去重排序，仅保留 ID 及 `source=connection_discovery`，不透传额外上游字段、错误正文、URL 或凭证。

**持久化缓存**：当发现结果为 `success` 时，服务端将本次发现的模型 ID 列表、分类、HTTP 状态、耗时与 RFC 3339 同步时间写入与 `models.json` 同目录的 `models-discovery-cache.json`（上限 2 MiB，不保存 URL/请求头/凭证，不修改 `models.json` 或 `agent.toml` 的 revision）。临时发现失败不会覆盖上次成功缓存；删除供应商时自动清理对应缓存条目。

| state | models | 含义 |
| --- | --- | --- |
| `success` | 数组（允许空） | 上游返回结构有效的模型列表；不等于真实调用成功或能力已验证 |
| `unsupported` | `null` | Adapter 未实现发现，或端点返回 404 / 405 / 501 |
| `failed` | `null` | 认证、限流、传输、超时、响应超限或其他 HTTP 错误 |
| `unknown` | `null` | 并发繁忙、响应无法识别、无效条目或声明存在未读取分页 |

结果另有 `category`、`http_status` 和 `elapsed_ms`；未启用、未配置凭证、连接不存在或 revision 冲突走现有配置 API 错误。发现失败不影响历史 Route，发现未知模型也保留其 ID。

## Web 模型管理、精选白名单与本地覆盖

供应商详情内联展示模型管理面板，切换供应商或刷新页面时自动加载持久化的最近一次同步缓存并显示上次同步时间：

- **模型精选白名单**：点击「管理 / 选择模型」打开宽档批量选择弹窗，聚合已发现模型、Catalog 参考模型与本地自定义模型，支持搜索、按系列自动分组、弹窗内触发「同步上游模型」、按组/全选当前筛选/仅保留已发现/清空与一键批量保存。精选白名单持久化在 `config/models.json` 的可选字段 `selections` 中（按连接 ID 索引，向后兼容旧文件），仅控制界面与路由快捷下拉的保留集合，不作为 Route 强制白名单，不会阻断已有路线。主列表支持一键「移出」及恢复显示全部。
- **模型元数据与来源**：模型名称与原始模型 ID 分开显示；目录补充要求模型 ID 精确匹配。标准身份映射：`openai → openai`、`deepseek → deepseek`、`gemini → google`、`bigmodel → zhipuai`。自定义 Connection 只补充 `models.json` 中以 canonical Connection ID 为 provider 的本地条目，不根据 URL 或品牌猜测目录身份。Advertised 是声明，Verified 始终标记未知；discovery 或 Catalog 不会自动改变 LLM 请求参数或工具权限。
- **本地新增 / 编辑 / 删除模型**：表单直接写入 `config/models.json`（`MODELS_CONFIG_FILE` 可指定部署路径），复用 mutation lock、opaque revision、文件安全检查和原子替换。开启精选白名单时，手动新增的本地模型会自动纳入精选列表。已添加的本地自定义模型支持直接「删除模型」，目录模型的本地覆盖支持「清除覆盖」。
- **模型路由联动**：既可在模型行一键「加入 Route」，也可在文本/STT 路由编辑器中通过「从已保留模型选择…」下拉框直接选取已启用供应商的保留模型（搜索路线同步提供 datalist 自动补全），并保留手动输入任意 `provider:model` 的能力。

模型管理 API 均要求管理员 Session、Origin 与 CSRF：

- `POST .../configuration/providers/model-metadata`：接收 `id`，返回 `metadata`，含 models revision、明确 provider 身份、有效元数据、当前本地 overrides、持久化 `discovered` 缓存、`selection_configured`、`selected_models`、目录出处、`verified_capabilities=unknown` 与 `apply_mode=restart`。不向上游请求。
- `PATCH .../configuration/providers/model-override`：接收 `id`、`expected_revision` 与 `LocalModelEntry` 格式的 `model`，校验后 CAS 原子写入。未知字段、Credential / Adapter / 业务权限字段及伪造 verified 能力均拒绝。
- `PUT .../configuration/providers/model-selection`：接收 `id`、`expected_revision`、`selected_models`（字符串数组保存精选白名单，`null` 清除精选限制恢复显示全部），去重校验后 CAS 原子写入 `models.json`。
- `DELETE .../configuration/providers/model-override`：接收 `id`、`expected_revision`、`model_id`，删除对应的本地覆盖/自定义条目并同步从精选列表中移除。

## disabled 模型引用完整性

本地显式标记 `enabled=false` 的模型不能被任何保存的模型路线或搜索路线引用，包括未使用路线和候选链中的非首项。禁用已引用模型、给路线添加已禁用模型、手工编辑后的 runtime / Secret / Agent 保存与启动 preflight 均拒绝非法配置，首次设置模式也不放宽此约束。保存失败不写入候选文件；已有非法路线可通过显式启用模型或修改路线修复。

目录缺失、发现失败、生命周期 deprecated、能力 unknown 或未勾选入精选白名单均不构成禁用，不会阻断既有路线运行。
