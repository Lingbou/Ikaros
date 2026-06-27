# RAG Model

Ikaros RAG indexes local files. Remote vector databases are not required for the MVP.

## Backends

Default JSONL path:

```text
IKAROS_HOME/rag/index.jsonl
```

SQLite path:

```text
IKAROS_HOME/rag/index.sqlite
```

Generated config uses local hash embeddings by default, so local indexing works
without provider credentials:

```yaml
rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small
```

Remote embeddings are opt-in. Configure provider settings when you select a
remote provider:

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

## Ingestion

RAG ingestion:

- walks files and directories
- skips `.git`, `target`, `node_modules`, and protected reference material
- chunks text by line windows
- stores source path and line metadata
- redacts secret-like content before indexing
- supports scope filtering
- can detect stale files
- can delete by scope or source path

Model-facing `rag_ingest` and `rag_reindex` skills walk workspace paths and read
file text through the session `ExecutionEnv` filesystem interface. The RAG
backend receives already-read source text plus metadata and is responsible for
index storage, embedding, search, stale checks, and deletion.
`rag_stale` reads indexed source metadata from the backend, then checks current
workspace metadata through `ExecutionEnv`; it does not let the tool path inspect
host files outside the harness boundary.

`ikaros-rag` is intentionally network-free. It owns local chunk storage,
retrieval, and local embedding primitives (`hash`, `sparse`, `mock`). Remote
embedding providers are constructed only by RAG skills after harness approval,
and execute through the session `ExecutionEnv` / `NetworkEgress` boundary.

Common commands:

```bash
ikaros rag ingest docs --scope project
ikaros rag search "harness policy"
ikaros rag stale
ikaros rag reindex docs --scope project
ikaros rag delete-path docs/old.md
ikaros rag delete-scope scratch
```

## Embeddings

Local deterministic/test providers:

- `hash`
- `sparse`
- `mock`

Local HTTP provider:

- `ollama`

Optional cloud provider:

- `openai-compatible`

`ollama` calls a local Ollama `/api/embed` endpoint. Leave
`providers.embedding.base_url` empty to use `http://127.0.0.1:11434`, or set it
to another local Ollama base URL. `openai-compatible` is the cloud embedding
provider name. Provider endpoints are configured through
`providers.embedding.base_url`, not through provider-name aliases.

OpenAI-compatible and Ollama embedding calls are network actions and require
harness approval for ingest, reindex, and search. After approval, model-facing
RAG skills replay the original request and route provider-backed embedding HTTP
through the session `ExecutionEnv` / `NetworkEgress` boundary. Text is redacted
before provider calls. Approval payloads describe the provider call, local file
read, and RAG index write scope without storing API keys. Tests use explicit
local/mock providers and do not require credentials.

RAG search output is intentionally rendered without raw embedding vectors. The
local index may store vectors, but CLI and skill output expose only the chunk
content, citation metadata, score, and embedding provider.

## Chat Context

Chat does not inject RAG by default. Local RAG can be added to a turn when a
profile enables `rag_context` and the request uses a nonzero `--rag-top-k`, or
when the user calls `ikaros rag search` directly. Provider-backed embedding
remains an explicit RAG command path rather than background chat retrieval.
