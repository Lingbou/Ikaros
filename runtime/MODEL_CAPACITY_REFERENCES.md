# 模型容量与运行限制参考

核对日期：2026-09-07。模型容量按实际模型与服务端点确定，不能把 Codex 的窗口数值直接用于其他模型。

## Ikaros 模型默认值

| 模型 ID | 上下文窗口 | Ikaros 初始输出预留 |
| --- | ---: | ---: |
| `deepseek-v4-flash` | 1,000,000 | 64,000 |
| `deepseek-v4-pro` | 1,000,000 | 64,000 |
| `deepseek-v4-flash-vision-exp` | 1,000,000 | 64,000 |
| 未识别的模型 | 32,768 | 4,096 |

[DeepSeek 官方模型表](https://api-docs.deepseek.com/quick_start/pricing/)为这三个模型标注 **1M 上下文、384K 最大输出能力**。64,000 是 Ikaros 为单次请求选择的初始输出预留，**不是 DeepSeek 官方默认值，也不是模型的最大输出能力**。官方表未公布默认输出值；384K 不在此擅自换算成精确整数。

[DeepSeek API 参考](https://api-docs.deepseek.com/api/create-chat-completion/)说明 `max_tokens` 控制一次生成的最大长度，输入与输出合计受上下文窗口约束。Ikaros 会从可用输入预算中扣除输出预留。用户可以在模型设置中调整两项数值；经第三方代理访问时，应以该端点实际限制为准。

上述值仅用于缺少容量字段的配置和新发现的模型。重新发现模型保留已有的手动容量，显式配置提交仍允许修改容量；不会自动重写既有配置。

## Hermes 运行上限与上下文策略

参考 [NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent) commit `d9833c5615b80e199a174cd67d90ab430695a972`：

- [当前配置默认值](https://github.com/NousResearch/hermes-agent/blob/d9833c5615b80e199a174cd67d90ab430695a972/hermes_cli/config_defaults.py#L49-L59)：`agent.max_turns = None`，默认不设迭代上限；`run_budget_seconds = None`，默认关闭总时长预算。独立的 `gateway_timeout = 1800` 针对无活动，不是任务运行 30 分钟就停止。
- [官方配置文档](https://hermes-agent.nousresearch.com/docs/user-guide/configuration/#iteration-budget)：支持主动设置迭代次数，以及 YAML/CLI 设置可选时间预算。时间预算到 80% 时提醒模型收尾，并调整静默超时。该页上方还残留“默认 500”的旧描述；这里采用同页后文与当前源码一致的“默认无限”。没有据此断言其桌面 UI 是否存在某个设置按钮。
- [容量解析实现](https://github.com/NousResearch/hermes-agent/blob/d9833c5615b80e199a174cd67d90ab430695a972/agent/model_metadata.py#L1844-L1939)：显式配置优先，再使用服务端点信息、Provider 元数据、本地探测、模型表；未识别时回退到 256,000 并警告。
- [压缩默认配置](https://github.com/NousResearch/hermes-agent/blob/d9833c5615b80e199a174cd67d90ab430695a972/hermes_cli/config_defaults.py#L529-L538)：全局比例为 50%；窗口小于 512,000 时触发比例至少为 75%。这属于 Hermes 策略，不代表模型本身的上下文规格。
