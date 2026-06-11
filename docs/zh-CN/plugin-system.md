# 插件系统

插件是位于以下目录的本地 manifest：

```text
IKAROS_HOME/skills
```

插件系统偏保守。Discovery 和 validation 是只读的。命令型插件只通过 harness 执行。

## Manifest 位置

支持：

- `IKAROS_HOME/skills/<plugin>/plugin.toml`
- `IKAROS_HOME/skills/<plugin>.toml`
- 可选 marketplace metadata：`IKAROS_HOME/skills/marketplace.toml`

## Manifest 示例

```toml
name = "hello"
version = "0.1.0"
description = "Example plugin."

[[skills]]
name = "echo"
description = "Echo JSON input."
risk = "safe_read"
input_schema = { type = "object", properties = { message = { type = "string" } } }

[skills.command]
program = "bin/echo.sh"
args = ["--json"]
timeout_ms = 1000
```

## CLI

```bash
ikaros skill list
ikaros skill audit
ikaros skill validate ./plugins/hello
ikaros skill install ./plugins/hello
ikaros skill inspect hello.echo
ikaros skill enable hello
ikaros skill disable hello
ikaros skill run hello.echo --input-json '{"message":"hello"}'
ikaros skill uninstall hello
```

## 规则

- 无效 manifest 会作为 warning 报告，不会加载。
- Plugin 和 skill 名称使用简单 ASCII identifier。
- Command 相对于 plugin manifest 目录。
- Command 不能使用 `..` 或受保护参考材料路径。
- 安装的插件默认 disabled，除非显式 enable。
- Declaration-only skill 可以 inspect，但不能执行。
- Command-backed skill 通过 `plugin_command_run` 执行。
- Plugin input/output 在 audit/reporting 前脱敏。
- 声明的 risk 仍然经过 policy。

CLI 永远不直接执行 plugin code。
