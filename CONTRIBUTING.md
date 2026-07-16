# Contributing

Ikaros stays intentionally small. A change belongs in the project only when it
strengthens the terminal chat, provider, approval, tool, or session path.

## Before submitting

```bash
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked
```

Also run `cargo deny check` and `cargo audit` when those tools are installed.

## Design rules

- Do not add compatibility layers for the removed pre-MVP architecture.
- Keep one execution path instead of parallel abstractions.
- Every side-effecting model tool requires visible user approval.
- Persist tool arguments before approval and never silently replay an
  interrupted tool.
- Keep file operations inside the selected workspace.
- Do not call host-process execution a sandbox.
- Never add real credentials to source, tests, fixtures, logs, or docs.
- Add focused behavior tests; avoid snapshots and abstractions that do not
  protect the current product.
