# Agent Loop

The agent loop is the model-guided execution path in `ikaros-runtime`. It lets a model request harness skills, receive tool results, and continue until it returns a final answer or hits a stop condition.

The loop owns turn orchestration. It does not own provider authentication,
provider wire format, policy decisions, or host execution. Those responsibilities
belong to `ModelProvider`/`ModelTransport` and the harness.

## Scope

The loop is small by design:

- bounded iteration count
- provider-native tool calls when available
- fallback JSON tool-call parsing for non-native output
- deterministic repair for common JSON shapes
- harness skill dispatch
- prompt-free audit metadata
- guardrail observation

The model never executes tools directly. Every tool call is normalized and sent through `ExecutionSession`.

## Interface

`AgentRuntime::run_turn()` receives:

- `AgentLoopInput`: optional task id, system prompt, and user input.
- `ModelProvider`: the configured provider adapter.
- `ExecutionSession`: policy, approval, audit, and environment context.
- `SkillRegistry`: executable harness skills.
- `AgentLoopOptions`: iteration, sampling, streaming, and guardrail settings.

The default implementation is `HarnessAgentRuntime`. Callers that need a
different loop implementation should swap the runtime layer, not the provider
adapter.

Default options:

- `max_iterations = 4`
- `max_tokens = 512`
- `temperature = 0.2`
- `stream = false`
- default guardrail settings

## Turn Sequence

Each iteration follows the same order:

1. Build a model request with system prompt, user input, prior assistant output,
   tool definitions, and prior tool results.
2. Ask the provider for a normal or streaming response.
3. Prefer provider-native tool calls when present.
4. If no native tool call exists, parse the fallback JSON protocol from text.
5. If a final answer is present, stop with `FinalAnswer`.
6. Dispatch normalized tool calls through `ExecutionSession`.
7. Append tool results to the next model turn.
8. Observe guardrails and iteration budget before continuing.

Provider-native tool call ids are preserved when the provider supplies them, so
tool result history can be sent back in the provider's preferred shape.

## Stop Reasons

The loop can stop because:

- a final answer was produced
- the iteration budget was reached
- policy denied a requested tool
- a requested tool needs approval
- a guardrail halted execution

Task and agent commands can opt into the loop with `--agent-loop`. Non-stream chat uses it by default; `--no-agent-loop` forces a single provider call.

Structured reports use these stop reasons:

- `FinalAnswer`
- `IterationBudget`
- `PolicyDenied`
- `WaitingForApproval`
- `GuardrailHalt`

Provider transport errors, malformed provider responses, local store errors, and
unexpected execution errors are returned as command errors rather than encoded
as normal stop reasons.

## Tool Calls

Preferred path:

1. Provider receives native tool definitions.
2. Provider returns native tool calls.
3. Runtime normalizes them.
4. Harness dispatches them.

Fallback path:

```json
{"tool_calls":[{"name":"tool_name","input":{}}]}
```

Final answer:

```json
{"final_answer":"..."}
```

The parser also accepts a few common variants such as fenced JSON, top-level arrays, `tool_call`, `function_call`, `args`, and `arguments`. Each iteration records the parse strategy in the report.

Parse strategies reported by the loop are:

- `provider_native_tool_calls`
- `direct_json_object`
- `direct_json_array`
- `fenced_json`
- `embedded_json_object`
- `embedded_json_array`
- `plain_text`

Strategies that required repair are marked with `repaired = true` in
`tool_call_diagnostics`.

## Report Contract

`AgentLoopReport` contains:

- stop reason
- final content
- provider and model names
- token usage
- whether streaming was used
- stream chunks when streaming is enabled
- iteration count
- tool-call diagnostics
- tool results

Tool result summaries and outputs are produced by the harness. They should be
redacted before surfacing to users or audit output.

## Invariants

- The prompt may describe tools, but only the harness registry defines what can
  be executed.
- Tool definitions include name, description, input schema, and risk level.
- A denied or approval-waiting tool call stops the loop; the loop does not try a
  different tool to bypass policy.
- Guardrails observe repeated failures and lack of progress after each tool
  dispatch.
- The fallback JSON protocol is compatibility behavior. Provider-native tool
  calls remain the preferred path.
