# Roadmap

This roadmap keeps the pre-MVP scope intentionally narrow. Ikaros is a
terminal-first local agent runtime, not a web/API platform.

## Current Focus

- Make `ikaros` and `ikaros chat` the primary interactive surface.
- Keep host assembly, session persistence, execution policy, providers, memory,
  RAG, approvals, and coding workflow on one coherent path.
- Prefer fewer, better-tested workflows over adjacent surfaces that do not help
  the terminal product.

## Next Milestones

1. Harden the terminal workbench: redraw behavior, multiline editing, session
   resume, timeline/replay navigation, approval handling, cancellation, and
   coding workflow controls.
2. Harden provider/context/sandbox diagnostics: provider fallback metadata,
   context compression evidence, network egress reporting, and local execution
   isolation checks.
3. Harden the coding workflow: provider-backed patch loops, rollback from
   persisted diffs, focused test evidence, cancellation, and replay inspection.
4. Harden MCP and plugins through the same `ExecutionSession`, approval, audit,
   workspace scope, and network egress boundaries as built-in skills.

## Out Of Scope For Now

- Local API servers, ACP, web dashboards, browser/CDP automation, standalone web
  search/extract surfaces, vision/image generation commands, voice providers,
  gateway/message daemons, schedule/automation workers, service managers, and
  self-modify commands.
- New work in those areas should wait until the terminal-first workflow is
  dependable end to end.

## Release Gates

- `cargo fmt --check`, `cargo check --workspace --all-targets`, and focused
  smoke tests pass locally.
- README and docs describe only the current product surface.
- `config.yaml` validation rejects unknown or removed feature branches.
- The top-level CLI help stays small enough that a new user can see the real
  local workflow at a glance.
