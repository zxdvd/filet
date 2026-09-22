# Operation and recovery notes

## State ownership

The data directory contains `state.sqlite3`, its WAL sidecars and `executor.lock`. `sources`, `observations`, `jobs`, `plans` and `operations` are separate tables. Schema version 1 is initial; a newer schema is rejected. Back up the stopped data directory as a whole. There are no destructive automatic migrations or retention cleanup in this release.

`plan` may add a plan while the daemon runs. Actual execution requires the same data-directory lock. Plan IDs are hashes of the input snapshot, chosen rule, source, complete revision and resolved action list. Re-applying a succeeded plan returns its stored state. A stopped operation does not silently create a second job.

Scans stream at most 256 directory entries per source per tick. Observations live in SQLite, so the candidate list does not need to fit in memory. A coalescing dirty bit makes duplicate events harmless and triggers a fresh scan. Ready candidates are rechecked at 250 ms intervals; event-free sources use `reconcileEvery`. Interval schedules coalesce after a restart and never replay every missed period. Continuous churn can defer a scan cycle; this is a best-effort foreground scheduler.

Initial baselining is incremental, not an atomic directory snapshot. Producers should remain paused during the very first baseline if the distinction between existing and newly arriving files matters. Initial source configuration changes create a fresh baseline. Rule-only revisions do not automatically replay observed event-based inputs. Scheduled rules may evaluate their existing scope under the new revision.

## Action progress

1. Persist intent and expected input.
2. Recheck input identity, size, modification time, available creation time and SHA-256.
3. Record started; publish without replacing a destination.
4. Persist the destination receipt before source deletion.
5. Recheck the source, remove it for a move, and flush the parent on Unix.
6. Record succeeded and suppress precisely identified self-generated observations.

Copies use bounded buffers, preserve ordinary permissions and modification time, and verify bytes. They do not promise ACLs, extended attributes, birth time, alternate streams, sparse layout or platform-specific file flags. Copy-based moves retain the source if publication or verification fails. Target temporary files are intentionally preserved when a step becomes uncertain.

Hard-link publication requires filesystem support. A filesystem without it receives an explicit failure, even if a less safe move would have worked. Same-filesystem hard-link/unlink moves are two steps, not atomic renames. Parent-directory durability is flushed on Unix; full Windows power-loss durability has not been established. This release targets process-crash recovery, with power-loss fault testing still a release gate.

## Review procedure

Use `filet history --json` to inspect the plan, each operation's expected input/output identity, paths, checksums, temporary files and command output. Stop the daemon before manually reconciling uncertain files. Preserve both files until their contents and provenance are confirmed. A same-content foreign destination is not proof that Filet created it.

On restart, a moved file whose native identity and content match the recorded input, whose source is absent, and whose destination is present can be acknowledged without moving again. A copy is acknowledged only with a recorded destination identity or a still-present matching temporary identity. A move with both paths present is left for review. Any interrupted external command is left for review, even if it may never have started.

Failed and uncertain built-in jobs block automatic work for their recorded source and destination paths. There is intentionally no `--force`, automatic rollback, generic undo or “retry uncertain exec” command. After manually reconciling a failed experiment, use a separate new data directory for a deliberately new workflow; do not treat deleting the state directory as a retry mechanism while a daemon is running. A richer reviewed resolution workflow is a productization follow-up.

## Process and script budgets

| Resource | Current bound |
|---|---|
| Config source | 1 MiB; nesting 24; 256 rules |
| Local JS package | 128 modules; 1 MiB/module; 4 MiB total |
| Fresh JS runtime | 32 MiB QuickJS heap; 512 KiB stack; 2 s execution/microtask budget |
| Action results | 64 actions; 1 MiB JSON |
| External command | 30 s default; maximum 1 h; 256 arguments |
| Captured output | 64 KiB for each of stdout/stderr; extra bytes drained |

JS memory limits use QuickJS's default allocator; the `rust-alloc`/custom-allocator features are not enabled. Promises have a bounded host-driven job loop. No synchronous host APIs perform unbounded extraction/network work. `check` still evaluates module top-level code within these budgets; it does not invoke rule entry points or execute returned actions.

Unix commands run in their own process group; normal descendants are terminated after exit or timeout. Windows commands are assigned to a kill-on-close Job Object. Assignment follows process creation, leaving a short launch race; detached Unix sessions and descendants created before Windows job assignment are outside the cleanup guarantee. Do not run self-daemonizing tools. The executor does not sandbox arbitrary commands or constrain their writes.

## Boundaries requiring operational care

Filet prevents destination replacement, but it cannot freeze the filesystem. Another process can replace path components or rewrite a file between a final check and an unlink. Symlink/reparse checks are repeated; they are not handle-relative protection against a hostile local process racing the directories. Use directories controlled by the same trusted user and a producer publication protocol (temporary filename then final rename) when stronger readiness is needed.

Native file I/O can block on the operating system, especially unavailable network mounts and cloud placeholders. Do not put sources/state on network or synced mounts for this release. Native hashing/copying uses bounded memory but has no hard wall-clock interruption. The SQLite database must be local. Cheap scan observations use native identity/size/mtime; changes that deliberately preserve all three may require an explicit manual plan to detect. Manual plans always hash content.

The CLI's `test` currently evaluates actual fixture files. A future fixture interface may accept fixed synthetic metadata and a frozen clock. `doctor` resolves statically declared YAML executables; dynamic JS executables are resolved in each generated plan. The [macOS guide](../skills/filet/references/macos.md) includes a LaunchAgent generator and manual service setup. A built-in service installer, log rotation, database retention/migrations, all-platform disk-full/cross-volume/power-cut testing and throughput baselines are still productization work.
