# Live vertical-slice validation

The original process-Tool vertical slice was validated on 2026-08-12
(Asia/Shanghai) on Windows, starting from Gate 6 commit `48b8cd5`. The most
recently recorded extension of the same end-to-end smoke with Provider-reported
Token usage, the file Tools, and an enabled frozen Skill descriptor passed on
2026-08-15. This document records that run; later code revisions require a
fresh opt-in run before they can be described as live-validated.

## Live path

The opt-in test at
`ikaros/desktop/src/renderer/runtimeStore.live.test.ts` exercises the production
path rather than calling the Provider adapter directly:

```text
Renderer store
  -> RuntimeClient bridge
  -> Desktop RuntimeHost
  -> Python Runtime
  -> enabled Skill descriptor frozen into the Run context
  -> DeepSeek OpenAI-compatible SSE
  -> write -> read -> edit -> read
  -> model.usage_recorded -> usage.read
  -> Runtime events
  -> Renderer projection
```

The most recently recorded file-Tool live run used the explicitly configured
`deepseek-chat` model. Its asserted evidence was:

- two sequential Turns in one Thread;
- one real enabled Skill discovered through the renderer Store and frozen into
  the first DeepSeek Run, while the Skill body remained lazy and absent from
  the persisted descriptor;
- prior-Turn context reaching the second Provider request;
- streamed assistant deltas;
- four real DeepSeek-requested Tool Calls in the exact order `write`, `read`,
  `edit`, `read`, with four completed Tool Results;
- the initial token being written, read, replaced by an edited token, and read
  again, with the edited bytes independently verified on disk;
- the prior-Turn marker and final edited token returned by the model in its
  final answer after the last Tool Result;
- completed Run, Item, and renderer projections;
- exact Provider-reported usage persisted for the live model Steps, with
  positive lifetime and peak totals, a non-null longest-task duration, active
  streaks, and daily buckets whose sum exactly matched the lifetime total;
- Desktop projections for successful `write`, `read`, and `edit` Tool activity;
- Stop issued through the renderer store against a real `process_run` execution
  requested by the deterministic ScriptedProvider; and
- a nested, hidden Windows child process terminated with that cancelled Run.

In that observed run, one real DeepSeek response also emitted assistant
narration before Tool Calls, and the Runtime preserved the narration and calls
as one Provider assistant step. This is observed-run evidence, not a
deterministic assertion that future upstream responses will use that ordering.
Offline regression tests cover preservation of the combined assistant step,
SQLite rebuild behavior, exact event ordering, and rejection of text emitted
after a completed Tool Call.

The test is skipped unless `IKAROS_LIVE_DEEPSEEK_SMOKE=1`. The credential is
read inside the test from `IKAROS_LIVE_DEEPSEEK_KEY_FILE`; its value is never an
argument or environment variable.

## Credential evidence

The live credential was persisted through `provider.configure` inside a unique
temporary Runtime home. The live smoke never uses the user's normal
`%USERPROFILE%/.ikaros` directory, and removes its temporary Runtime home after
the credential checks complete.

The following checks passed without printing or hashing the credential:

- captured live-test stdout and stderr did not contain it;
- Runtime stderr captured by `RuntimeHost` did not contain it;
- JSON-RPC responses, replayed events, Provider/Model summaries, and the
  renderer store snapshot did not contain it;
- `SqliteRuntimeStore.journal_contains_protected_values` returned false;
- raw-byte scans of the temporary `state.db*`, `runtime.lock`, and every other
  file in the isolated Runtime home returned no match;
- the temporary `config.yaml` contained the expected matching value and was
  removed with the isolated Runtime home;
- the source credential file supplied for validation was preserved because its
  deletion was not authorized.

The user's existing Provider configuration and conversation database are not
read, modified, or removed by the live smoke.

Repository secret scanning is not part of the live smoke. It is handled as a
separate read-only repository audit and is not claimed as live-test evidence
here.

## Re-running

From `ikaros/desktop` in PowerShell:

```powershell
$env:IKAROS_LIVE_DEEPSEEK_KEY_FILE = "<path-to-key-file>"
pnpm.cmd run test:live:deepseek
Remove-Item Env:IKAROS_LIVE_DEEPSEEK_KEY_FILE
```

Ordinary `pnpm test` remains deterministic and offline; it collects this test
as skipped.
