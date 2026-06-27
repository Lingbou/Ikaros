# RAG 模型

Ikaros RAG 索引本地文件。MVP 不需要远程向量数据库�?

## 后端

默认 JSONL 路径�?

```text
IKAROS_HOME/rag/index.jsonl
```

SQLite 路径�?

```text
IKAROS_HOME/rag/index.sqlite
```

生成配置默认使用本地 hash embedding，因此本地索引不需�?provider 凭证�?

```yaml
rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small
```

远程 embedding 需要显式启用。选择远程 provider 时配置对�?provider 设置�?

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

## 摄取

RAG ingestion 会：

- 遍历文件和目�?
- 跳过 `.git`、`target`、`node_modules` 和受保护参考材�?
- 按行窗口切分文本
- 存储 source path �?line metadata
- 在索引前脱敏疑似 secret 内容
- 支持 scope 过滤
- 检�?stale file
- �?scope �?source path 删除

面向模型�?`rag_ingest` �?`rag_reindex` skill 会通过 session `ExecutionEnv`
filesystem interface 遍历 workspace path 并读取文件文本。RAG backend 接收已经读取�?
source text �?metadata，负�?index 存储、embedding、search、stale 检测和删除�?
`rag_stale` 会先�?backend 读取已索�?source metadata，再通过 `ExecutionEnv`
检查当�?workspace metadata；tool 路径不能绕过 harness 边界直接检�?host 文件�?

`ikaros-state::rag` 刻意保持无网络能力。它只拥有本�?chunk 存储、检索和本地 embedding
primitive（`hash`、`sparse`、`mock`）。远�?embedding provider 只能�?RAG skill �?
harness 审批后构造，并通过当前 session �?`ExecutionEnv` / `NetworkEgress` 边界执行�?

常用命令�?

```bash
ikaros rag ingest docs --scope project
ikaros rag search "harness policy"
ikaros rag stale
ikaros rag reindex docs --scope project
ikaros rag delete-path docs/old.md
ikaros rag delete-scope scratch
```

## Embedding

本地 deterministic/test provider�?

- `hash`
- `sparse`
- `mock`

本地 HTTP provider�?

- `ollama`

可�?cloud provider�?

- `openai-compatible`

`ollama` 会调用本�?Ollama `/api/embed` endpoint。`providers.embedding.base_url`
留空时使�?`http://127.0.0.1:11434`，也可以显式设置为其他本�?Ollama base URL�?
`openai-compatible` �?cloud embedding provider 名称。Provider endpoint 通过
`providers.embedding.base_url` 配置，不通过 provider-name alias 表达�?

OpenAI-compatible �?Ollama embedding call 都是网络动作，ingest、reindex �?search
都需�?harness 审批。审批后，面向模型的 RAG skill 会重放原�?request，并通过当前
session �?`ExecutionEnv` / `NetworkEgress` 边界执行 provider-backed embedding HTTP�?
文本�?provider 调用前脱敏。审�?payload 会说�?provider call、本地文件读取和 RAG
索引写入范围，但不会保存 API key。测试显式使用本�?mock provider，不需要凭证�?

RAG search 输出不会暴露原始 embedding vector。本地索引可以保存向量，�?CLI �?skill output 只展�?chunk 内容、citation metadata�?
score �?embedding provider�?

## Chat 上下�?

Chat 默认不注�?RAG。只�?profile 启用 `rag_context` 且本轮使用非�?`--rag-top-k`，或者用户直接执�?`ikaros rag search` 时，
才会把本�?RAG 作为�?citation �?reference retrieval 使用。Provider-backed embedding 仍然通过显式 RAG 命令触发，不做后台聊天检索�?
