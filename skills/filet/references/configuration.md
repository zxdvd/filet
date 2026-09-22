# Configuration and validation

## Schema v1

```yaml
schemaVersion: 1
sources:
  inbox:
    path: ./inbox
    recursive: false
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
      - copy:
          to: ./backup
          onConflict: error
      - rename:
          name: archived.pdf
      - move:
          to: ./archive
```

Resolve source paths, action destinations, script paths, and exec working directories against the configuration's directory. Resolve CLI config and input paths against the shell working directory. Support `~` for home paths; do not invent `$HOME`/environment interpolation inside YAML. Prefer absolute paths for services. Keep rule IDs unique.

Use `.mjs` or `.js` with a `script` block instead of both `when` and `actions`:

```yaml
    script:
      file: ./rules/archive.mjs
      apiVersion: 1
```

Reject unknown/duplicate fields, YAML tags, and merge keys. Use positive integer durations with `ms`, `s`, `m`, `h`, or `d`. Set `retryFor >= stableFor`.

## Matching and sources

| Field | Meaning |
|---|---|
| `extension` | ASCII lowercase without dot, e.g. `pdf` |
| `nameGlob` | Glob against the filename |
| `minSizeBytes`, `maxSizeBytes` | Inclusive byte limits |
| `modifiedOlderThan`, `createdOlderThan` | Age at the fixed evaluation time; unavailable timestamps do not match |
| `all`, `any`, `not` | Nested conditions; multiple fields on one condition are ANDed |

Use `on: {type: file.ready, source: inbox}` for changed, stable inputs. It has no `every`. For a periodic rule use `on: {type: scan, source: inbox, every: "1h"}`. A scan deliberately includes existing files and re-evaluates unchanged files each interval; a copy-only or exec-only scan may repeat effects across intervals. Prefer moving completed inputs out of the source when that fits the user's intent.

Default source exclusions are `**/*.crdownload`, `**/*.part`, `**/*.tmp`, and `**/.filet-*`. Specifying `ignore` replaces the list; retain the defaults when adding patterns. Set `recursive: true` explicitly when subdirectories should be included. Never nest/overlap source roots. Reject symlink inputs and symlink/reparse path components. Keep SQLite on a local, nonsynced disk.

## Actions

Use `copy: {to: DIRECTORY}`, `move: {to: DIRECTORY}`, or `rename: {name: BASENAME}`. YAML defaults the file reference to the input. Only `error` (default) and `skip` conflict policies exist. A skipped copy permits later actions; a skipped move/rename ends the plan.

For an external command, use the final action only:

```yaml
      - exec:
          program: /absolute/path/to/tool
          args:
            - --input
            - pathOf: input
          cwd: ./work
          env:
            PATH: /usr/bin:/bin
          timeout: "30s"
```

Supply literal arguments: no shell expansion, pipes, or interpolation. The child starts with an empty environment (plus `SystemRoot` on Windows). Supply HOME, PATH, locale, or other variables only as required. Resolve executables when planning; prefer absolute executable paths in unattended rules. External commands run with the user's permissions.

## Fixtures and diagnostics

Create actual fixture files and a JSON array. Resolve each fixture path relative to the JSON file. Ensure the fixture configuration's sources contain those files.

```json
[
  {"path": "inbox/sample.pdf", "expectedRule": "archive-pdfs", "expectedActionCount": 3},
  {"path": "inbox/sample.txt", "expectedRule": null, "expectedActionCount": 0}
]
```

`test` reads file metadata and evaluates plans without executing actions or persisting state. It does not inspect file contents for semantic PDF validity, compare all action arguments, or supply a synthetic clock. Use `plan` to inspect resolved actions. For time predicates create fixture timestamps deliberately and avoid tests at wall-clock thresholds.

Use `doctor --json` for source accessibility, SQLite integrity, and YAML executable resolution. For dynamic JS commands, inspect a generated plan. Use `history --limit 20 --json` for jobs and step receipts. Pass the same `--data-dir` every time; otherwise apparently missing history may simply be a different database.
