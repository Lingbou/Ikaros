# 插件系统

插件是位于以下目录的本地 manifest：

```text
IKAROS_HOME/skills
```

插件系统偏保守。Discovery 和 validation 是只读的。命令型插件只通过 harness 执行。

## Manifest 位置

支持：

- `IKAROS_HOME/skills/<plugin>/plugin.toml`
- 可选 marketplace metadata：`IKAROS_HOME/skills/marketplace.toml`

`IKAROS_HOME/skills/<name>/SKILL.md` 是另一类 artifact。它会被加载为
prompt skill document，而不是可执行 plugin。当前 toolset 允许时，模型可以通过
`tool_search` 发现它，通过 `tool_describe` 读取它，但 `tool_call` 永远会拒绝把它
当成可执行工具。搜索结果只暴露 provenance、support-file 名称等 descriptor metadata，
不会返回 instruction body 或 support-file 内容；`tool_describe` 会按需加载这些内容，
并在返回给模型前脱敏疑似 secret。

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

非 `safe_read` 的命令型 skill 必须声明匹配的运行时权限。`action` 可以是
`run`、skill 名、完整 skill 名，或 `plugin:<完整 skill 名>`；`risk` 必须和
skill 自身的 risk 一致。

```toml
[[skills]]
name = "write-report"
description = "Write a report under the workspace."
risk = "local_write"

[[skills.permissions]]
action = "run"
risk = "local_write"
paths = ["reports/output.md"]

[skills.command]
program = "bin/write-report.sh"
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
ikaros skill quarantine hello --reason "needs operator review"
ikaros skill unquarantine hello
ikaros skill run hello.echo --input-json '{"message":"hello"}'
ikaros skill uninstall hello
```

## 规则

- 无效 manifest 会作为 warning 报告，不会加载。
- Plugin 和 skill 名称使用简单 ASCII identifier。
- `SKILL.md` prompt skill document 使用目录名作为 skill name，并可以在 front matter
  中声明 `description`、`toolset`、`provenance` 和安全相对路径形式的 `support_files`。
  安全 support file 只会被 `tool_describe` 按需加载，不会出现在搜索结果或默认 prompt
  注入里。
- Command 相对于 plugin manifest 目录。
- Command 不能使用 `..` 或受保护参考材料路径。
- 安装的插件默认 disabled，除非显式 enable。
- Quarantined plugin 仍然保留安装记录，也能通过 `inspect`/`audit` 查看，但可执行
  skill 会从运行时和模型可调用集合中隐藏，直到执行
  `ikaros skill unquarantine <name>` 解除隔离。
- Declaration-only skill 可以 inspect，但不能执行。
- Command-backed skill 通过 `plugin_command_run` 执行。
- 高于 `safe_read` 风险的命令型 skill 如果没有匹配的运行时权限声明，会被拒绝执行。
- Permission path 必须位于 workspace 内，且不能指向 `.temp`。
- `network` 和 `remote_server` 插件 skill 必须使用 `network = true` 的匹配权限声明。
- Plugin input/output 在 audit/reporting 前脱敏。
- 声明的 risk 仍然经过 policy。

CLI 永远不直接执行 plugin code。
