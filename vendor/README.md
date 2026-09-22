# Crossterm 0.29.0: bounded Unix Escape ambiguity

This is the published MIT-licensed Crossterm 0.29.0 source, copied from the verified
Cargo registry cache (archive SHA-256
`d8b9f2e4c67f833b660cdb0a3523065869fb35570177239812ed4c905aeff87b`).
Every recorded source hash was checked against that archive. Its license is retained at `crossterm/LICENSE`.
The workspace `[patch.crates-io]` selects this copy for the CLI, Ratatui and textarea;
no transitive runtime versions change. Do not modify the machine's registry cache.

Only `src/event/source/unix/mio.rs` differs from upstream. The exact text patch is
`crossterm-escape.patch`; `crossterm-upstream-sha256.json` records original file hashes.
Registry bookkeeping and the upstream development lockfile were omitted.

The active Unix mio backend now retains a lone Escape for 250 ms across short polls,
and caps a blocking poll at that deadline. It returns to readiness polling instead
of issuing a blocking second read while Escape is pending. Continuations arriving before the deadline
use the existing byte parser. A lone Escape emits once at expiry; repeated Escape
presses remain distinct. An established sequence/paste is not subject to this timer.
The public API, Windows backend and unused `use-dev-tty` backend are unchanged.
This is not an arbitrary-delay guarantee: a suffix after 250 ms is ordinary input.
It does not repair unterminated-paste buffering or general malformed sequences.

Why this patch: upstream 0.29.0 immediately decodes a lone Escape at a read boundary.
Akasha cannot safely recover original paste bytes after receiving decoded key events.
An application sleep cannot change the parser's decision. Replacing the terminal stack
would be a substantially larger change. Revisit this patch when an upstream release
provides equivalent bounded behavior and passes the same regression tests.

Verification:

```sh
cargo test --manifest-path vendor/crossterm/Cargo.toml --lib --no-default-features --features events,bracketed-paste akasha_escape_tests
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build -p akasha-cli
python3 scripts/test-tui-pty.py
python3 scripts/test-tui-pty.py --profile keyboard --split-paste-start
```

The standalone vendor tests resolve their own upstream development dependencies;
the product build and PTY checks use the workspace lockfile. The blocking-input test
uses a disposable Unix socketpair; a sandbox must permit local socket writes. Local test artifacts
and that standalone lockfile are ignored. Vendored upstream code retains its existing
`unused_parens` warning; the application still passes warnings-denied Clippy.

Upstream references (reviewed 2026-09-22):
- https://github.com/crossterm-rs/crossterm/tree/0.29
- https://github.com/crossterm-rs/crossterm/blob/master/src/event/source/unix/mio.rs
