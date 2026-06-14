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

本地 provider：

- `hash`
- `sparse`
- `mock`

可选 cloud provider：

- `openai-compatible`

`openai-compatible` 是唯一的 cloud embedding provider 名称。Provider endpoint 通过 `providers.embedding.base_url` 配置，不通过 provider-name alias 表达。

Cloud embedding call 是网络动作，ingest、reindex 和 search 都需要 harness 审批。文本在 provider 调用前脱敏。测试显式使用本地/mock provider，不需要凭证。

RAG search 输出不会暴露原始 embedding vector。本地索引可以保存向量，但 CLI 和 skill output 只展示 chunk 内容、citation metadata、score 和 embedding provider。

## Chat 上下文

自动 chat context lookup 使用 SafeRead 本地 RAG。Cloud embedding 通过显式 `ikaros rag` 命令触发，而不是自动聊天检索。
