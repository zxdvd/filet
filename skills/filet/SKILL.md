---
name: filet
description: Install and operate Filet file automation; author and validate filet.yaml rules and sandboxed ESM JavaScript using @app/api. Use when configuring watched folders, copy/move/rename/exec workflows, scheduled rules, plans, history, or the Filet daemon and macOS launchd service.
---

# Filet

Manage Filet through its CLI and ordinary configuration files. Target schemaVersion 1 and JS apiVersion 1. Treat the installed binary's `--help`, `schema --json`, and validation results as authoritative when versions differ from this skill.

## Read the relevant reference

- Read [configuration.md](references/configuration.md) for YAML, matching, scheduling, actions, and fixture files.
- Read [javascript.md](references/javascript.md) before writing any JS rule or helper.
- Read [macos.md](references/macos.md) for installation, first use, launchd, updates, and troubleshooting on a Mac. Use the bundled `scripts/launch_agent.py` to generate a plist with correctly escaped absolute paths.

## Establish the workflow

1. Locate the installed `filet`, its version, configuration, rule package, and data directory. Inspect existing files and state before changing them. Keep the same explicit `-c` and `--data-dir` throughout a workflow.
2. Establish the user's intended source, destination, filename policy, conflict behavior, and whether to process existing files. Use session context; ask only when a missing decision changes the result materially.
3. Prefer YAML for declarative filters and fixed actions. Choose JS for computed names, branching, or multiple derived actions. Keep configuration, JS packages, state, logs, and destinations outside watched sources. Use local ordinary files and nonoverlapping sources.
4. Make edits in a staging configuration beside the live configuration so relative paths retain their meaning. Keep incomplete JS edits outside the active package. Remember that a running daemon activates valid edits automatically and immediately executes matching plans.

## Validate, review, execute

1. Run `filet check -c CONFIG --data-dir STATE --json`. This validates package exports but does not invoke rule functions; do not treat it as a rule behavior test.
2. Create an isolated fixture configuration with sources and destinations in a disposable directory, outside all live watched roots. Test a match, a nonmatch, and relevant boundary cases with `filet test -c FIXTURE_CONFIG FIXTURES --data-dir FIXTURE_STATE --json`. Assert rule IDs and action counts; inspect `plan` output for actual paths and arguments.
3. Run `filet plan -c CONFIG INPUT --data-dir STATE --json` against the intended input. Inspect `data.plan` (possibly null), its `planId`, chosen rule, resolved actions, destinations, and commands. This writes the plan to SQLite but executes no actions.
4. Apply within the user's authorized scope using `filet apply -c CONFIG PLAN_ID --data-dir STATE --json`. Stop the daemon first: its execution lock prevents separate applies. If files, config, or JS helpers changed since planning, generate a new plan. Never bypass a stale-plan failure.
5. Inspect `history --data-dir STATE --json`. After verification, publish complete configuration changes and start/resume `daemon` when ongoing automatic execution is in scope. Explain what the active rules do and which existing files they include.

Use JSON's `ok`, `errors`, and process exit status. Report the actual result, changed paths, active configuration, data directory, and whether the daemon is running. `status` shows persisted state, not process liveness; on macOS use `launchctl print` as well.

## Preserve these contracts

- Use the first nonempty matching plan only. Do not assume later rules also run or errors fall through.
- Expect the first `file.ready` scan to baseline existing files without processing them. Use explicit `plan`/`apply` for a chosen historical file; choose `scan` only when periodic processing of existing files is intended. Keep producers paused during the initial baseline if the boundary matters.
- Keep `to` as a directory and `name` as one filename. Use only `onConflict: error` or `skip`; there is no overwrite policy. Copy retains the original FileRef; move and rename advance it.
- Keep external `exec` last. Use literal argv and `pathOf` for the current file after a move. Supply required environment variables explicitly. Do not silently add a shell or unrelated commands.
- Treat `needs_review` and interrupted commands as review work. Preserve state and file receipts; do not delete SQLite, invent a force/retry command, or rerun a potentially completed external effect. Inspect both source and destination before resolving an uncertain operation.
- Expect no general undo and no preservation guarantee for ACLs, extended attributes, or Finder tags across copies. Report limitations that matter to the requested workflow.

## Example requests

- “Install Filet on my Mac and run it at login.” Follow the macOS reference, verify a disposable example, then enable the LaunchAgent if requested.
- “Archive PDFs from this inbox by year and month.” Read the JS reference; choose the timestamp from the user's intent, return a move action, and inspect concrete plans for dated and missing-date inputs.
- “Why did nothing happen to an old file?” Check initial baseline semantics, actual config/data paths, source availability, matching order, and history before changing rules or state.
