# 模型容量与运行限制参考

核对日期：2026-10-01。模型容量按实际模型与服务端点确定，不能把 Codex 的窗口数值直接用于其他模型。

## Ikaros 模型默认值

| 模型 ID | 上下文窗口 |
| --- | ---: |
| `deepseek-v4-flash` | 1,000,000 |
| `deepseek-v4-pro` | 1,000,000 |
| `deepseek-v4-flash-vision-exp` | 1,000,000 |
| 未识别的模型 | 32,768 |

[DeepSeek 官方模型表](https://api-docs.deepseek.com/quick_start/pricing/)为这三个模型标注 **1M 上下文、384K 最大输出能力**。
Ikaros 不把 384K 作为用户设置，也不在主执行请求中发送 `max_tokens`；输出上限由
模型和 Provider 决定。输入预算使用上下文窗口的 95%，自动压缩阈值使用 90%。

[DeepSeek API 参考](https://api-docs.deepseek.com/api/create-chat-completion/)说明 `max_tokens` 控制一次生成的最大长度，输入与输出合计受上下文窗口约束。经第三方代理访问时，应以该端点实际限制为准；上游拒绝输出参数时，Ikaros 不要求用户修改模型容量来绕过协议错误。

上述上下文值仅用于缺少容量字段的配置和新发现的模型。重新发现模型保留已有的手动
容量，显式配置提交仍允许修改上下文窗口；不会自动重写既有配置。
