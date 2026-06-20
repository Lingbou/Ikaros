# RAG 模型

Ikaros RAG 索引本地文件。MVP 不需要远程向量数据库。

## 后端

默认 JSONL 路径：

```text
IKAROS_HOME/rag/index.jsonl
```

SQLite 路径：

```text
IKAROS_HOME/rag/index.sqlite
```

生成配置使用远程 provider 形状，并把 provider 设置留空。这样缺少 cloud 配置时会尽早报错，而不是回退到 mock provider：

```yaml
providers:
  embedding:
    api_key: ""
    base_url: ""

rag:
  backend: jsonl
  embedding_provider: openai-compatible
  embedding_model: ""
```

如果要完全本地索引、不使用 provider 凭证，需要显式选择本地 embedding provider：

```yaml
rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small
```

## 摄取

RAG ingestion 会：

- 遍历文件和目录
- 跳过 `.git`、`target`、`node_modules` 和受保护参考材料
- 按行窗口切分文本
- 存储 source path 和 line metadata
- 在索引前脱敏疑似 secret 内容
- 支持 scope 过滤
- 检测 stale file
- 按 scope 或 source path 删除

面向模型的 `rag_ingest` 和 `rag_reindex` skill 会通过 session `ExecutionEnv`
filesystem interface 遍历 workspace path 并读取文件文本。RAG backend 接收已经读取的
source text 和 metadata，负责 index 存储、embedding、search、stale 检测和删除。
`rag_stale` 会先从 backend 读取已索引 source metadata，再通过 `ExecutionEnv`
检查当前 workspace metadata；tool 路径不能绕过 harness 边界直接检查 host 文件。

常用命令：

```bash
ikaros rag ingest docs --scope project
ikaros rag search "harness policy"
ikaros rag stale
ikaros rag reindex docs --scope project
ikaros rag delete-path docs/old.md
ikaros rag delete-scope scratch
```

## Embedding

本地 deterministic/test provider：

- `hash`
- `sparse`
- `mock`

本地 HTTP provider：

- `ollama`

可选 cloud provider：

- `openai-compatible`

`ollama` 会调用本地 Ollama `/api/embed` endpoint。`providers.embedding.base_url`
留空时使用 `http://127.0.0.1:11434`，也可以显式设置为其他本地 Ollama base URL。
`openai-compatible` 是 cloud embedding provider 名称。Provider endpoint 通过
`providers.embedding.base_url` 配置，不通过 provider-name alias 表达。

OpenAI-compatible 和 Ollama embedding call 都是网络动作，ingest、reindex 和 search
都需要 harness 审批。文本在 provider 调用前脱敏。测试显式使用本地/mock provider，不需要凭证。

RAG search 输出不会暴露原始 embedding vector。本地索引可以保存向量，但 CLI 和 skill output 只展示 chunk 内容、citation metadata、score 和 embedding provider。

## Chat 上下文

Chat 默认不注入 RAG。只有 profile 启用 `rag_context` 且本轮使用非零 `--rag-top-k`，或者用户直接执行 `ikaros rag search` 时，才会把本地 RAG 作为带 citation 的 reference retrieval 使用。Provider-backed embedding 仍然通过显式 RAG 命令触发，不做后台聊天检索。
