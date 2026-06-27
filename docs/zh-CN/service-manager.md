# 服务管理器模板

Ikaros 可以为本地 worker 进程渲染 service-manager 模板。渲染不会安装、enable、reload 或启动服务。

## 渲染目标

```bash
ikaros service render --kind schedule-worker --manager systemd
ikaros service render --kind message-worker --manager systemd
ikaros service render --kind message-webhook --manager launchd
```

常用选项：

```bash
ikaros service render \
  --kind message-worker \
  --manager systemd \
  --interval-seconds 30 \
  --limit 10 \
  --output services/ikaros-message-worker.service
```

`--output` 限制在 `IKAROS_HOME` 下。

## 模板内容

模板包含本地 runtime 参数，例如：

- `--ikaros-home`
- `--workspace`
- 可选 `--agent`
- worker interval 和 limit
- webhook host 和 port

Systemd `message-worker` 模板会保持 `ExecStart` 为前台 `message worker`
进程，让 service manager 直接管理进程本身。同时它会增加 `ExecStop` hook，执行
`message worker-stop --reason "service manager stop"`，给 worker 一个短的协作式关闭窗口，
让 gateway worker 消费 `message-worker.stop` 并写入 shutdown forensics。

模板不包含 API key。当 worker 触发 provider 调用时，会从选中的本地 `IKAROS_HOME/config.yaml` 读取 `providers.*` 设置。
