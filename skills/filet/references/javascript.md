# JavaScript API v1

Write ESM, not Node.js. Import the virtual module `@app/api`; do not install it from npm. Use `export const apiVersion = 1` and exactly one entry-point style: `run(ctx)` or `match(ctx)` plus `actions(ctx)`. Sync and Promise-returning functions work. Return an actual boolean from `match`; return an action array from `actions`. Return `undefined`, `null`, or `[]` from `run` for no match. Exceptions are failures, not no-match results.

## Context

Treat `ctx` and its nested objects as immutable:

| Property | Value |
|---|---|
| `ctx.file.ref` | Opaque input reference, currently `"input"` |
| `ctx.file.name`, `stem`, `extension` | Filename, stem, lowercase extension without dot |
| `ctx.file.sizeBytes` | File length |
| `ctx.file.modifiedAt`, `createdAt` | Timestamp strings or null |
| `ctx.file.path` | Unicode path or null; avoid using it for arguments after moves |
| `ctx.event.reason` | `changed`, `reconcile`, `scheduled`, or `manual` |
| `ctx.now` | Fixed RFC 3339 evaluation time |

Use `ctx.now` for time-dependent decisions, not a fresh wall-clock reading. Avoid random decisions so a reviewed plan remains explainable. No file content reader, hashing function, or persistent JS state is exposed.

## Computed paths

This example organizes PDFs by their modification month in UTC, with the evaluation time as a fallback. Choose that policy explicitly for the user's workflow.

```js
import { actions as a } from "@app/api";
export const apiVersion = 1;

export function run(ctx) {
  if (ctx.file.extension !== "pdf") return;
  const timestamp = ctx.file.modifiedAt ?? ctx.now;
  const month = new Date(timestamp).toISOString().slice(0, 7);
  return [
    a.copy(ctx.file.ref, { to: "./backup", onConflict: "error" }),
    a.move(ctx.file.ref, { to: `./archive/${month}`, onConflict: "error" }),
  ];
}
```

`to` is always a directory. To change the filename, use `a.rename(ctx.file.ref, {name: "new-name.pdf", onConflict: "error"})`. Copy keeps the current reference at its input; rename and move update it. Derive filenames as single path components and validate user-derived fragments.

Alternatively use:

```js
import { actions as a } from "@app/api";
export const apiVersion = 1;
export function match(ctx) { return ctx.file.extension === "pdf"; }
export function actions(ctx) {
  return [a.move(ctx.file.ref, { to: "./archive" })];
}
```

## External commands

Append at most one exec, in the final position:

```js
a.exec({
  program: "/absolute/path/to/tool",
  args: ["--input", a.pathOf(ctx.file.ref)],
  cwd: "./work",
  env: { PATH: "/usr/bin:/bin" },
  timeout: "30s",
})
```

`pathOf` is a late-bound argument resolved to the current file after preceding moves/renames. Do not stringify it or interpolate it into a longer string. Use separate argv entries. JS prepares action data; it does not execute commands during evaluation. Exec results are not available to that JS evaluation. Do not model OCR/content extraction followed by a JS decision as a single plan; that requires a separate, explicitly designed workflow.

## Modules and limits

Put each rule package in a dedicated directory outside watched roots. The entry module's parent is the package root. Import local helpers with relative paths, including extensions. The entire `.js`/`.mjs` package tree is snapshotted and hashed; editing any helper invalidates unstarted plans. Do not import above the root or through symlinks.

Expect no `require`, npm resolution, Node built-ins, filesystem APIs, network APIs, timers, remote imports, or TypeScript transpilation. A Promise can use the runtime's microtask queue, but cannot await nonexistent host I/O. Use plain `.mjs` and local pure helpers.

Budget each fresh evaluation for 32 MiB heap, 512 KiB stack, and 2 seconds of execution/microtasks. Return no more than 64 actions or 1 MiB JSON. Package limits are 128 modules, 1 MiB per module, and 4 MiB total. Keep top-level code bounded too: `check` loads modules but does not call their rule functions.

Consult `types/api-v1.d.ts` in the repository for editor typing. It is a declaration aid; no Node or npm runtime is needed. Validate all examples with the installed Filet binary rather than with Node alone.
