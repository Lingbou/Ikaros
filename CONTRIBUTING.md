# Contributing

Thanks for helping build Ikaros.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Use Conventional Commits for future commits, but do not create commits in automation unless the maintainer explicitly asks.

## Pull Requests

- Keep changes scoped to the owning crate or document the cross-crate reason.
- Add tests for policy, memory, RAG, provider, and CLI behavior when those surfaces change.
- Do not include real secrets in tests, docs, fixtures, logs, or audit samples.
- Do not modify local reference-material directories; they are not part of the project.
- Update docs when behavior, security boundaries, configuration, or CLI commands change.

## DCO / CLA

No CLA is configured. The intended policy is Developer Certificate of Origin sign-off for external contributions once the project is opened publicly.
