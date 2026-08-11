# Live vertical-slice validation

Validated on 2026-08-12 (Asia/Shanghai) on Windows, starting from Gate 6 commit
`48b8cd5`.

## Live path

The opt-in test at
`ikaros/desktop/src/renderer/runtimeStore.live.test.ts` exercises the production
path rather than calling the Provider adapter directly:

```text
Renderer store
  -> RuntimeClient bridge
  -> Desktop RuntimeHost
  -> Python Runtime
  -> DeepSeek OpenAI-compatible SSE
  -> process.run
  -> Runtime events
  -> Renderer projection
```

The final live run passed with the explicitly configured `deepseek-chat` model
and proved:

- two sequential Turns in one Thread;
- prior-Turn context reaching the second Provider request;
- streamed assistant deltas;
- one real `process_run` Tool Call;
- a command-generated GUID that was absent from the prompt returned to the model
  before its final answer;
- completed Run, Item, and renderer projections;
- Stop issued through the renderer store;
- a nested, hidden Windows child process terminated with the cancelled Run.

The test is skipped unless `IKAROS_LIVE_DEEPSEEK_SMOKE=1`. The credential is
read inside the test from `IKAROS_LIVE_DEEPSEEK_KEY_FILE`; its value is never an
argument or environment variable.

## Credential evidence

The live credential was persisted through `provider.configure`. Within
Ikaros-owned state, its only permitted persistent location is
`%USERPROFILE%/.ikaros/config.yaml`.

The following checks passed without printing or hashing the credential:

- captured live-test stdout and stderr did not contain it;
- Runtime stderr captured by `RuntimeHost` did not contain it;
- JSON-RPC responses, replayed events, Provider/Model summaries, and the
  renderer store snapshot did not contain it;
- `SqliteRuntimeStore.journal_contains_protected_values` returned false;
- raw-byte scans of `state.db*`, `runtime.lock`, temporary files, and every
  other Ikaros-owned file returned no match;
- raw-byte scans of repository tracked and untracked files returned no match;
- `config.yaml` contained exactly one matching value;
- the source credential file supplied for validation was preserved because its
  deletion was not authorized.

On this machine, `icacls` reported only the current user, `SYSTEM`, and
`Administrators` on `config.yaml`; no broad `Everyone`, `Users`, or
`Authenticated Users` grant was present.

## Re-running

From `ikaros/desktop` in PowerShell:

```powershell
$env:IKAROS_LIVE_DEEPSEEK_KEY_FILE = "<path-to-key-file>"
pnpm.cmd run test:live:deepseek
Remove-Item Env:IKAROS_LIVE_DEEPSEEK_KEY_FILE
```

Ordinary `pnpm test` remains deterministic and offline; it collects this test
as skipped.
