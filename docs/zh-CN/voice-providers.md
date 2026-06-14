# 语音 Provider

语音调用通过 `ikaros-voice`。

## Provider

已实现：

- `mock`：确定性的本地 TTS/ASR provider，用于显式离线测试。
- `openai-compatible`：`/audio/speech` 和 `/audio/transcriptions` adapter。

Cloud voice provider 只接受 `openai-compatible`。这个名称选择 wire format；配置的远端服务必须实际提供对应的 TTS 或 ASR endpoint。

示例：

```yaml
providers:
  tts:
    api_key: "replace-with-your-tts-key"
    base_url: "https://api.example.com/v1"
  asr:
    api_key: "replace-with-your-asr-key"
    base_url: "https://api.example.com/v1"

voice:
  tts:
    provider: openai-compatible
    model: provider-tts-model
    voice: alloy
  asr:
    provider: openai-compatible
    model: provider-asr-model
```

## 安全

- Mock provider 不需要凭证或网络，但必须显式选择。
- Cloud provider 从本机 `IKAROS_HOME/config.yaml` 读取明文 key 和 base URL。
- TTS 文本在 provider 调用前脱敏。
- Cloud voice call 是网络动作；active policy 对网络设为审批时会先返回 approval request。
- `voice tts --output <path>` 是 workspace 写入，在策略要求时需要审批。
- ASR transcript 输出不应回显 source path。

TTS 成功输出只报告 provider、format、可选输出路径、字节长度和脱敏文本预览，不打印原始音频字节。ASR 以 multipart form data 发送音频文件，CLI 只渲染 transcript metadata。

语音 schema 携带 audio path、format、sample rate 和 language 元数据。Adapter 使用 provider 支持的字段，不支持的字段保留为 Ikaros 侧元数据。
