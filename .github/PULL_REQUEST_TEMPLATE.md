## Summary

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check --locked --all-targets`
- [ ] `cargo clippy --locked --all-targets -- -D warnings`
- [ ] `cargo test --locked --all-targets`
- [ ] `cargo build --release --locked`
- [ ] Focused manual smoke, if applicable:

## Safety

- [ ] Tool side effects remain approval-gated.
- [ ] Interrupted tools are not automatically replayed.
- [ ] No real secrets were added.
- [ ] No removed legacy surface or compatibility layer was reintroduced.
