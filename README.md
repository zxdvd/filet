# filet

Local file automation you can inspect before it runs. Simple rules use YAML; complex rules use ESM JavaScript. Both produce the same versioned action plans.

**Status: experimental 0.1.** This implements the CLI-first MVP from [the architecture review](docs/architecture-review-v0.2.md). It is intended for testing on disposable files before enabling real workflows. There is no GUI, Node.js runtime dependency, OCR, cloud model integration, or universal undo.

**macOS:** follow the [installation and launchd guide (中文)](skills/filet/references/macos.md) to build, validate a workflow, and run at login. `filet daemon` is a foreground process; launchd provides background supervision.

**Agent setup:** the repository includes a portable [Filet skill](skills/filet/SKILL.md) covering YAML, JavaScript, plan validation, operation, and recovery. Ask your local agent to read that file, or copy the entire `skills/filet` folder into the skill directory supported by your agent. The reference files and plist generator travel with it. For example:

> Read `skills/filet/SKILL.md` in this checkout. Install Filet on my Mac, verify a disposable PDF archive workflow, and configure it to run at login. Use `~/.config/filet/filet.yaml` and `~/Library/Application Support/filet/state`. Report the final rules, validation results, and service status.

## Build

Install Rust and a native C toolchain (GCC/Clang on Linux/macOS, MSVC on Windows):

```sh
cargo build --release --locked --bin filet
cargo test --workspace --locked
cargo install --locked --path crates/filet-cli
```

Rust is pinned in `rust-toolchain.toml`; Rust dependencies are pinned in `Cargo.lock`. QuickJS and SQLite are compiled into the application. CI runs native tests on Ubuntu 24.04 x64, macOS 14 arm64, and Windows Server 2022 x64. Platform support is provisional until those jobs pass on the committed revision. Release candidate builds are manually triggered CI artifacts, not automatic public releases.

## Quick start

Create an Inbox beside your configuration. Paths in rules are relative to that configuration, including action destinations and command working directories.

```yaml
schemaVersion: 1
sources:
  inbox:
    path: ./inbox
    ready:
      stableFor: "3s"
      retryFor: "10m"
    reconcileEvery: "10m"
rules:
  - id: archive-pdfs
    on:
      type: file.ready
      source: inbox
    when:
      extension: pdf
    actions:
      - move:
          to: ./archive
          onConflict: error
```

```sh
filet check -c filet.yaml --json
filet plan -c filet.yaml ./inbox/example.pdf --json
# Copy data.plan.planId from the previous result:
filet apply -c filet.yaml <plan-id> --json
filet history --json
```

`plan` reads and hashes the file and persists its plan. It never moves files or starts commands. `apply` uses the saved action data, checks the current source fingerprint and complete rule-package revision, and does **not** evaluate JavaScript again. Editing the config or a helper module invalidates unstarted plans.

Use `--data-dir ./filet-state` consistently to isolate a workflow's state. The default is the platform's local per-user data directory. Keep it on a local filesystem, outside watched and synced directories. A running daemon holds its execution lock; an independent `apply` exits with `INSTANCE_LOCKED`.

Run the foreground daemon only after reviewing the rules:

```sh
filet daemon -c filet.yaml
```

New `file.ready` sources establish a baseline of existing files. Use explicit `plan`/`apply` to process history. Ordinary restarts reconcile changes since the previous observations. `scan` rules explicitly opt into existing files and re-evaluate unchanged files on their interval. Events are hints; durable observations and bounded incremental scans also discover files after missed notifications. Stable size/mtime is a readiness heuristic, not a promise that a producer has finished writing.

## JavaScript

In the manifest, replace `when`/`actions` with:

```yaml
    script:
      file: ./rules/archive.mjs
      apiVersion: 1
```

```js
import { actions as a } from "@app/api";
export const apiVersion = 1;

export function run(ctx) {
  if (ctx.file.extension !== "pdf") return;
  return [
    a.copy(ctx.file.ref, { to: "./backup" }),
    a.rename(ctx.file.ref, { name: `archived-${ctx.file.stem}.pdf` }),
    a.move(ctx.file.ref, { to: "./archive" }),
  ];
}
```

Choose either `run(ctx)` or `match(ctx)` + `actions(ctx)`; synchronous and Promise-returning functions are supported. `match` must return an actual boolean. Empty `run` results do not claim the file. Errors never mean “no match.” Inputs are deeply frozen, and every evaluation has its own runtime.

The rule package is the script entry's parent directory. All `.js`/`.mjs` sources below it are snapshotted and hashed at activation (including helpers loaded with dynamic `import`). Only package-relative imports and the virtual `@app/api` module are allowed. No `require`, npm resolution, filesystem globals, timers, network globals, or remote modules are provided. Pending promises with no possible host work fail promptly. CPU execution, microtask loops, memory, stack, result size, and action count are bounded.

See [the API declarations](types/api-v1.d.ts), [YAML examples](examples/yaml), and [JS examples](examples/js). Declarations assist external editors/type checkers; the CLI does not embed TypeScript.

## Commands

| Command | Result |
|---|---|
| `check` | Schema, package load, API version and entry-point validation |
| `test <fixtures.json>` | Read-only rule/plan assertions on local fixture files |
| `plan <path>` | Persist a concrete plan with source content hash and native identity |
| `apply <plan-id>` | Revalidate and apply an existing plan; repeat calls reuse final status |
| `daemon` | Watch, scan, execute, conservatively recover and atomically reload valid configs |
| `status` | Persisted source availability and job counts |
| `history [--limit N]` | Job states, step receipts, errors and bounded command output |
| `doctor` | Source accessibility, database check and YAML executable resolution |
| `schema` | JSON schemas generated from the Rust configuration and plan types |

`--json` emits `{formatVersion, ok, data, errors}` on stdout. Daemon events and diagnostics go to stderr. Errors have stable codes, a message and, when known, a `ruleId`. YAML parse errors include source locations in the message. `status` and `history` remain available when the current config is invalid.

Try the bundled fixtures without changing files:

```sh
filet test -c examples/yaml/filet.yaml examples/yaml/fixtures.json --json
```

Each fixture declares `path` (relative to the fixture file), `expectedRule` (or `null`), and `expectedActionCount`. The `sample.pdf` fixture is ordinary text: these rules inspect metadata, not PDF contents.

## Execution contract

- Rules run in configuration order. The first nonempty plan claims the input. Failure does not fall through to another rule.
- `copy` keeps the current FileRef on the original. `rename` and `move` advance it. `to` is a directory; `name` is a single filename. Missing destination directories are created and recorded as part of their parent action's intent.
- `error` and `skip` are the only conflict policies. A skipped copy allows later actions; a skipped move/rename stops the remaining plan because its dependent concrete paths were not reached.
- Publication uses an atomic no-clobber hard link. Filesystems that cannot provide it fail closed. Same-filesystem moves publish a hard link then unlink the source; other moves copy to a unique destination temporary file, verify, flush, publish, verify again, and remove the source. No step calls an overwriting rename.
- Inputs are ordinary files. Directory actions, symlinks, junctions/reparse points, lossy Unicode paths, cyclic action targets, full ACL/xattr/alternate-stream preservation and permanent deletion are unsupported.
- SQLite WAL records intent, started/published steps and receipts. Recovery acknowledges provable completed operations. Ambiguous states retain files and become `needs_review`. Built-ins are serialized under one execution lock; unrelated programs do not obey that lock.
- File operations and SQLite are not one transaction. No universal undo or exactly-once external effects are promised. Read [recovery and operational limits](docs/operations.md) before unattended use.

## External commands

Return `a.exec({program, args, cwd, env, timeout})` as the **last** action. Use `a.pathOf(ctx.file.ref)` for a late-bound path argument:

```js
return [
  a.move(ctx.file.ref, { to: "./ready" }),
  a.exec({
    program: "my-tool",
    args: [a.pathOf(ctx.file.ref)],
    timeout: "30s",
  }),
];
```

The executable is resolved to an absolute path during planning. Arguments are passed literally, without a shell. `cwd` defaults to the config directory. The child environment starts empty and uses only `env` (plus `SystemRoot` on Windows); explicitly provide `PATH` if the child needs it. Windows `.bat`/`.cmd` executables require an explicit shell. Output is capped at 64 KiB per stream. Timeouts, host interruption and nonzero exit codes require review and are never automatically replayed. Scripts and executables are trusted user-installed code, not a malicious-code sandbox.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The macOS plist helper uses Python 3's standard library. Run `python3 -m unittest discover -s tests -p 'test_*.py'` to verify path escaping and validation without installing a service; macOS also validates the generated file with native `plutil`.

See [validation evidence and remaining release gates](docs/validation.md), [operations](docs/operations.md), and [architecture](docs/architecture-review-v0.2.md). This repository does not select an open-source license on the author's behalf.
