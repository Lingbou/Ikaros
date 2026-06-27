# Body Model

Body 层是展示边界：它展示 agent 当前状态，但不执行工具。

## 合约

Body wire 类型放在 `ikaros-protocol::body`：

- `BodyStatus`：当前 persona、emotion、task state、context count、policy decision 和路径信息。
- `BodyEvent`：脱敏后的事件项，`data` 使用 typed JSON value。
- `BodyFrame`：一个 status snapshot 加 recent events。

`ikaros-agent::body_status` 负责从 persona、task、chat、audit state 组装 body frame。CLI 渲染放在 `ikaros-terminal::body`，web dashboard 渲染放在 `ikaros-surfaces::body`。

## 规则

Body surface 可以展示 approval、audit path、task status、persona 和 emotion。它不能批准请求、执行 skill、写 memory、调用模型或绕过 execution policy。
