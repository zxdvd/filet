---
name: filet
description: Configure an installed Filet by writing or editing filet.yaml and JavaScript rules. Use for file matching, sorting, renaming, archiving, and custom file automation logic.
---

# Configure Filet

Assume Filet is installed. Read the existing configuration and the user's desired source folders, matching criteria, and destinations. Preserve unrelated rules. Use YAML for filters and fixed actions; use JavaScript for computed names, branching, and other custom logic.

## YAML

Start with this `filet.yaml` and adapt its paths and rules:

```yaml
schemaVersion: 1
sources:
  inbox:
    path: ./inbox
    recursive: false
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

Paths are relative to `filet.yaml`; `~` is supported, `$HOME` interpolation is not. Keep destinations outside watched folders and source folders nonoverlapping. Give each rule a unique ID. The first rule producing actions wins.

| Need | Configuration |
|---|---|
| Include subfolders | Set source `recursive: true` |
| Wait for writes to settle | Source `ready: {stableFor: "3s", retryFor: "10m"}` (defaults) |
| Match filename | `when: {nameGlob: "invoice-*.pdf"}` |
| Match size | `minSizeBytes` / `maxSizeBytes` (inclusive) |
| Match age | `modifiedOlderThan: "7d"` / `createdOlderThan: "7d"` |
| Combine conditions | `all: [...]`, `any: [...]`, `not: {...}`; sibling fields are ANDed |
| Copy or rename | `copy: {to: ./backup}` / `rename: {name: new-name.pdf}` |
| Periodic processing | `on: {type: scan, source: inbox, every: "1h"}` |

Use lowercase extensions without a dot. Durations use a positive integer plus `ms`, `s`, `m`, `h`, or `d`. `to` is a directory; `name` is one filename. Conflicts support only `error` (default) and `skip`, never overwrite.

A new `file.ready` source skips existing files on its first scan. Use `scan` when existing files should be processed periodically; unchanged inputs can match again each interval.

## JavaScript

Read [javascript.md](references/javascript.md) when YAML cannot express the rule. It covers connecting a `.mjs` file, the context, action helpers, and an example with computed destinations.

## Check the result

Validate the config and preview a representative file using the workflow's existing data directory:

```sh
filet check -c /path/to/filet.yaml --data-dir /path/to/state --json
filet plan -c /path/to/filet.yaml /path/to/inbox/sample.pdf --data-dir /path/to/state --json
```

`check` validates configuration and JS exports; `plan` evaluates the rule without executing its actions. For branching JS, preview inputs for the relevant branches. Report the changed rules and concrete destinations from the plans.

A running daemon hot-reloads valid edits. Validate a sibling draft config before replacing a live config; keep draft JS in a separate directory until ready.
