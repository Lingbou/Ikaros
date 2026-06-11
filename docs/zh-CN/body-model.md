# Body 模型

Body 层是展示边界。它渲染 runtime 状态，但不执行工具。

## 合约

`ikaros-body` 定义：

- `BodyStatus`：当前 persona、emotion、task state、context count、policy decision 和路径。
- `BodyEvent`：脱敏事件项。
- `BodyFrame`：一个 status snapshot 加 recent event。
- CLI 和 web dashboard 输出的 render adapter。

`ikaros-runtime` 从 persona、task、chat 和 audit state 组装 body frame。UI 代码应消费这些 frame，而不是重新实现 runtime 逻辑。

## 命令

```bash
ikaros body status
ikaros body dashboard
ikaros body dashboard --refresh-seconds 5 --snapshot-output previews/frame.json
ikaros body serve --port 8001
```

Dashboard 默认把本地 HTML 写到 `IKAROS_HOME` 下。Server 默认绑定 `127.0.0.1`，只提供只读 frame 数据。

## 规则

Body surface 可以展示 approval、audit path、task status、persona 和 emotion。它们不能批准请求、执行 skill、写 memory、调用模型或绕过 harness policy。
