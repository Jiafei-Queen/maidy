# 消息延迟收发

> 状态：当前设计基线，更新于 2026-11-22。

本文描述两条聊天节奏能力：连续消息的延迟接收（短窗口聚合）和 `send_message` 的拟人发送延迟。平台协议、Core 业务规则和 Tool 白名单不在这里重复。

## 延迟接收

延迟接收解决用户把一句话拆成多条消息发送时机器人抢答的问题。

### 范围

| 平台 | 私聊 | 群聊 |
| --- | --- | --- |
| QQ 官方 | 支持 | 不支持，仍立即分发 |
| OneBot 11 | 支持 | 不支持，仍立即分发 |
| 微信服务号 | 不支持 | 不适用 |

OneBot 11 的聚合发生在 `OneBotScopeDispatcher` 之前，等待期间不占用 scope worker；QQ 官方沿用现有 `MessageAggregator`。

### 判定规则

只有同时满足以下条件的消息进入聚合批次：

- 私聊；
- 非空纯文本；
- 不是引用消息；
- 不是 `/ping` 等 Gateway 本地命令；
- Core `classify_inbound` 返回 `NormalChat`。

命令、pending 确认、媒体消息和引用消息会先封口当前批次，再立即进入原有分发链路，保持同一会话内的顺序。

### 封口条件

| 条件 | 行为 |
| --- | --- |
| 最后一条消息后静默达到 `MESSAGE_AGGREGATION_QUIET_MS` | 封口并合并为一条逻辑消息 |
| 自批次第一条起达到 `MESSAGE_AGGREGATION_MAX_WAIT_MS` | 封口；配置为 `0` 表示不设硬上限 |
| 消息数达到 `MAX_MESSAGES` | 立即封口 |
| 字符数达到 `MAX_CHARS` | 立即封口 |
| Gateway 关闭 | flush 所有批次后再停止 worker |

同一用户连续发送时，每条新消息都会重置静默计时。合并后的文本按换行连接，输入片段、@ 和附件按原顺序拼接。

### 去重与失败

- 物理消息在进入聚合器前用现有 `MessageDedupe` 预留；重复消息直接忽略。
- 聚合器把预留令牌一并交给 `OneBotScopeDispatcher`，避免合并消息按末尾 `message_id` 二次预留。
- 投递失败时预留自动回滚，用户可以稍后重发；OneBot 会尝试发送一次失败提示，提示发送失败只记 debug。

## 延迟发送

延迟发送只作用于模型主动调用 `send_message` 的场景，不影响命令回复、最终文本兜底、状态提示和 `send_voice`。

### 配置

| 变量 | 默认 | 说明 |
| --- | --- | --- |
| `CHAT_SEND_DELAY_MIN_MS` | `1000` | 首条消息前随机延迟下限 |
| `CHAT_SEND_DELAY_MAX_MS` | `2000` | 上限；与下限相同即固定延迟 |
| `CHAT_SEND_GAP_MS` | `900` | 多条消息之间的间隔 |

`MIN=MAX=0` 关闭首条延迟；`GAP=0` 关闭条间停顿。

### 预算裁剪

延迟在调用平台投递前执行，因此会消耗工具剩余预算。Core 至少保留 3 秒真实发送时间：

```text
effective_delay = min(sampled_delay, remaining_budget - 3s)
```

剩余预算不足 3 秒时直接放弃本次延迟，优先保证消息送达。工具超时、取消和真实发送结果语义保持不变。

### 正在输入

- QQ 官方沿用现有原生 typing；`send_message` 不再重复发送。
- OneBot 11 在发送延迟期间并行调用 NapCat 扩展：

```json
{ "action": "set_input_status", "params": { "user_id": "<QQ号>", "event_type": 1 } }
```

- 只对私聊调用；群聊没有该状态。
- 调用失败只记 debug，不影响消息发送。
- 不主动清除输入状态，等 QQ 客户端自然超时，避免误发“正在说话”状态。
- 由 `ONEBOT11_AGENT_TYPING_ENABLED` 控制，默认开启。

## 配置中心

以上字段都注册为 `runtime.toml` 受管字段：

- 发送节奏：`chat.send.*`；
- 延迟接收：`chat.aggregation.*`；
- OneBot typing：`platform.onebot11.agent_typing_enabled`。

Web Console 的“回复与语音 → 消息节奏”分节可以直接修改。所有字段保存后需要重启生效；控制台会显示待重启数量，并提供重启按钮。`.env` 中已有值作为外部兜底，保存受管值后受管值优先。

## 不变量

- 聚合和延迟只改变时序，不改变去重、会话作用域、Tool 成功判定和发送结果。
- 不在 Gateway 中复制 Core 的命令或 pending 关键词规则。
- 日志只记录脱敏 scope、批次大小、等待时长、封口原因和错误分类，不记录聊天正文或完整平台 ID。
