# Self-Modify

Self-modify is a narrow proposal/apply workflow. It is not a general permission for the agent to rewrite itself.

## Flow

```bash
ikaros self-modify propose --kind documentation-patch --target README.md --diff "<unified diff>"
ikaros self-modify request-apply <proposal-id>
ikaros approval approve <approval-id>
ikaros self-modify apply-approved <proposal-id> --approval-id <approval-id>
```

Rollback:

```bash
ikaros self-modify rollback <proposal-id>
```

Inspection:

```bash
ikaros self-modify list
ikaros self-modify operations
ikaros self-modify heartbeat
```

## Guarantees

- `RiskLevel::SelfModify` is denied for normal tool dispatch.
- A proposal stores a redacted diff summary and rollback snapshot.
- Apply requires a dedicated approval id.
- The approval must match the proposal and workspace.
- The target must not have drifted from the snapshot.
- Restricted check commands run before and after apply.
- Failed pre-checks stop before mutation.
- Failed post-checks trigger rollback.
- Operations are recorded under local self-modify state.

## State

```text
IKAROS_HOME/self-modify/proposals.jsonl
IKAROS_HOME/self-modify/operations.jsonl
IKAROS_HOME/self-modify/rollback/<proposal-id>/target.snapshot
```

## Check Profiles

Built-in profiles use restricted test/check/lint/build commands. Config can override them:

```yaml
self_modify:
  check_profiles:
    runtime_patch:
      commands:
        - cargo check --workspace --all-features
      reason: "Runtime patches must keep the workspace compiling."
```

Allowed kinds:

- `skill_patch`
- `persona_patch`
- `config_patch`
- `runtime_patch`
- `documentation_patch`

Shell chaining, redirection, command substitution, publishing, and git commit/push/tag actions remain rejected.

## Not Implemented

Autonomous apply is not part of the current contract. The current version only supports explicit proposal, approval, apply-approved, and rollback flows.
