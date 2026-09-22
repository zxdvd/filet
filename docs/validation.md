# Validation

The initial implementation is verified in a Linux x64 workspace with Rust 1.94.0. Native Windows x64 and macOS arm64 results must come from CI; adding the workflow is not itself evidence that those targets passed.

The local acceptance run includes 38 passing integration/contract tests, successful YAML/JS example checks, fixture assertions, formatting, and a warnings-as-errors Clippy pass. The real daemon test exercises initial baselining, exclusive execution and preserving the active configuration after an invalid hot reload.

The automated contract suite covers:

- YAML duplicate keys, unknown fields, tags/merges, legacy boolean spellings and documented map-form actions.
- First nonempty plan, stable file references, copy-original semantics, rename/move sequences and repeated apply.
- Content changes with restored size/mtime, changed revisions, destination conflicts and no-clobber behavior.
- Native identities distinguishing independent same-content files; recovery of a completed move, incomplete publication and foreign destination ambiguity.
- Single-instance locking, initial baselines, restart reconciliation with missed events, scheduled re-evaluation of unchanged inputs and self-effect suppression.
- Async ESM, version/entry contracts, local static/dynamic imports, frozen contexts, fresh runtimes, invalid actions, timeout and effective memory limits.
- External command path binding, timeout, captured output and non-retry of uncertain effects (Unix fixtures).

Run the full gate:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked --bin filet -- check -c examples/yaml/filet.yaml --json
cargo run --locked --bin filet -- check -c examples/js/filet.yaml --json
cargo run --locked --bin filet -- test -c examples/yaml/filet.yaml examples/yaml/fixtures.json --json
```

Remaining release gates: native macOS/Windows CI results; Windows command-tree cleanup launch races; genuine cross-volume interruptions, disk full and power-loss tests; native notification behavior on common desktop editors; OS service installation/upgrade tests; history retention and future database migration backup/restore. No throughput, latency, idle memory, exactly-once or universal undo claims are made.
