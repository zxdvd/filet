# JavaScript rules

Replace a rule's `when` and `actions` with this block (keep its `id` and `on`):

```yaml
script:
  file: ./rules/archive.mjs
  apiVersion: 1
```

Create `rules/archive.mjs`. Use plain ESM JavaScript and the built-in `@app/api`; no npm installation is needed.

## Example: sort PDFs by category and month

```js
import { actions as a } from "@app/api";
export const apiVersion = 1;

export function run(ctx) {
  if (ctx.file.extension !== "pdf") return [];
  const category = /^invoice[-_]/i.test(ctx.file.stem) ? "invoices" : "other";
  const month = new Date(ctx.file.modifiedAt ?? ctx.now).toISOString().slice(0, 7);
  return [
    a.move(ctx.file.ref, { to: `./archive/${category}/${month}` }),
  ];
}
```

Return an array of actions; `[]`, `null`, or `undefined` means no match. Throwing means failure. The example uses modification month in UTC, falling back to evaluation time; adapt that policy to the user's request.

## Available context

| Property | Meaning |
|---|---|
| `ctx.file.ref` | Input reference for action helpers |
| `ctx.file.name`, `stem`, `extension` | Filename, stem, lowercase extension without dot |
| `ctx.file.sizeBytes` | Size in bytes |
| `ctx.file.modifiedAt`, `createdAt` | Timestamp strings or `null` |
| `ctx.file.path` | Original path or `null` |
| `ctx.now` | Fixed evaluation timestamp; use for time-based logic |
| `ctx.event.reason` | `changed`, `reconcile`, `scheduled`, or `manual` |

## Action helpers

```js
a.copy(ctx.file.ref, { to: "./backup", onConflict: "error" });
a.move(ctx.file.ref, { to: "./archive", onConflict: "skip" });
a.rename(ctx.file.ref, { name: "new-name.pdf" });
```

Return these values in the action array to use them. `to` is a directory relative to the YAML config; `name` is a single filename. Copy keeps the reference at the original file; move and rename update it. Conflicts accept only `error` (default) and `skip`.

For an external command, append one `exec` as the last action:

```js
a.exec({
  program: "/absolute/path/to/tool",
  args: ["--input", a.pathOf(ctx.file.ref)],
  env: { PATH: "/usr/bin:/bin" },
  timeout: "30s",
});
```

Use `a.pathOf` as a separate argument to resolve the current path after moves or renames. Arguments are literal, with no shell expansion. The child environment is empty except `SystemRoot` on Windows; supply needed variables explicitly. JS creates the plan and cannot read an exec result.

## Helpers and runtime

Import pure helpers with relative paths such as `./naming.mjs`; keep them under the entry module's directory. There is no Node.js, npm resolution, filesystem/content reader, network, or timer API. Use metadata and pure JS; return actions for file operations. Stay within 2 seconds, 32 MiB of heap, and 64 actions per evaluation. Validate with Filet's `check` and `plan`, not Node.
