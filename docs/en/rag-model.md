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

Generated config uses the remote-provider shape with empty provider settings.
This makes missing cloud configuration fail early instead of falling back to a
mock provider:

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

For fully local indexing without provider credentials, select a local embedding
provider explicitly:

```yaml
rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small
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

Local providers:

- `hash`
- `sparse`
- `mock`

Optional cloud provider:

- `openai-compatible`

`openai-compatible` is the only cloud embedding provider name. Provider
endpoints are configured through `providers.embedding.base_url`, not through
provider-name aliases.

Cloud embedding calls are network actions and require harness approval for
ingest, reindex, and search. Text is redacted before provider calls. Tests use
explicit local/mock providers and do not require credentials.

RAG search output is intentionally rendered without raw embedding vectors. The
local index may store vectors, but CLI and skill output expose only the chunk
content, citation metadata, score, and embedding provider.

## Chat Context

Automatic chat context lookup uses SafeRead local RAG. Cloud embedding is explicit through `ikaros rag` commands rather than automatic chat retrieval.
