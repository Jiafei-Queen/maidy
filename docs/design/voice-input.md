# 语音输入（OneBot 11 与 QQ 官方）

本文说明用户语音如何进入小女仆：平台自带转写如何优先、外部 STT 如何回退，以及音频、日志和配置的边界。配置字段与默认值以 [`runtime/config/agent.example.toml`](../../runtime/config/agent.example.toml) 和 [NapCat 接入说明](../development/onebot11-napcat.md) 为准。

## 目标与范围

- OneBot 11 用户语音可被识别并进入普通聊天。
- QQ 官方语音在平台没有可用转写时，可回退到外部 STT。
- 转录模型链可配置、可排序、可降级，形态与文本模型路由一致。
- 音频只在 Gateway 内处理；Core 与 LLM 只接收转写文本。

本期不做：本地 SILK 解码、内置 ffmpeg、流式转写、频道/微信语音、引用语音的 STT 回退。

## 候选链中的平台模型

平台自带转写不再是一个独立策略开关，而是候选链中的保留模型 `platform:qq`：

1. **`platform:qq`**：QQ 官方 `asr_refer_text`，OneBot/NapCat `fetch_ptt_text`。免费、不需要密钥，也不上传音频。
2. **远程候选**：OpenAI 兼容 `POST /audio/transcriptions`，按 `provider:model` 顺序降级。

同一条消息只执行一次候选链，命中即停止。平台文本为空或去掉标点后没有实际文字时，继续尝试下一个候选；把 `platform:qq` 放在远程候选之后，就会先调用远程 STT。从链路中删除 `platform:qq` 即停用平台转写。

控制台的“转录模型路由”为每条链路提供“添加 QQ 平台转录”按钮；链路已存在该候选时按钮禁用，不会写入重复项。

`[stt] enabled = false` 只跳过远程候选；`platform:qq` 仍可工作，因为平台文本不会外发音频。`[stt] platform_asr` 仅保留旧配置兼容：没有显式 `platform:qq` 且值为 `"auto"` 时，运行时会在链路前补一次平台转写；`"off"` 不补。控制台保存会统一写入 `"off"`，并只通过候选链表达平台转录。

## 配置

```toml
[stt]
enabled = false
# 旧配置兼容字段；新配置统一为 off，平台转录由 platform:qq 候选表达。
platform_asr = "off"
language = ""
timeout_seconds = 30
platform_asr_attempts = 2
max_audio_bytes = 10485760
max_text_chars = 2000

[stt.routes]
default = ["platform:qq", "openai:whisper-1", "bigmodel:glm-asr-2512"]
private = []
group = []
```

- 场景选择：私聊优先 `private`，群聊优先 `group`；场景链为空时回退 `default`。
- `enabled = true` 时至少要求 `default` 或某个场景链非空。
- 候选 Provider 复用内置连接或 `[providers.*]`；只接受 `kind = "openai_compatible"`，`openai_responses` 在保存/启动校验时拒绝。
- `platform:qq` 不需要 Provider 声明；其他 `platform:*` 模型会被拒绝。
- 缺钥匙不阻止启动：状态降级为 `missing_credentials`，远程候选跳过，平台候选仍可用。

## 音频来源与格式

| 平台 | 首选 | 回退 |
| --- | --- | --- |
| QQ 官方 | `voice_wav_url`（平台转好的 WAV） | 原始 `url`，嗅探后决定 |
| OneBot/NapCat | `get_record(out_format="mp3")` 返回的 `base64` | 原始 `record.url`，嗅探后决定 |

- 只使用 `get_record` 的 `base64`：它的 `file`/`url` 是 NapCat 本机路径，跨机不可信。
- 上传前按文件头嗅探容器；SILK 直接判失败，不发送给第三方。
- 音频不落盘、不进 Core/LLM；`max_audio_bytes` 同时约束下载与上传。

## 注入与失败语义

成功：

```text
[语音转文字] <识别文本>
```

来源为 `TextSource::Transcript`。失败：

```text
[语音消息：未识别出文字]
```

来源为 `TextSource::Context`。两者都只写入 `input_parts`，不写入 `inbound.text`，所以语音不会触发 `/todo` 等命令或 pending。

## 安全与日志

- 群聊语音仍只在 `@` 或引用机器人时处理。
- 日志只记录来源、provider、model、chain_index、duration_ms、audio_bytes、transcript_chars、error_class。
- 不记录转写文本、音频 URL、base64、message_id、openid 或群号。
- 远程 STT 默认关闭；`platform:qq` 只使用平台下发的文本，不会把音频发送到第三方。

## OneBot WebSocket 上限

`ONEBOT11_MAX_MESSAGE_BYTES` 的公开默认值仍是 1 MiB，事件帧继续按该值拒绝。启用远程候选时，传输层只对 **action response** 使用内部 20 MiB 上限，因为 NapCat `get_record` 的 base64 音频会走同一条反向 WebSocket 返回；应用层仍把非 action response 限制在配置值内。
