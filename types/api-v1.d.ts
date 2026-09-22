declare module "@app/api" {
  export type FileRef = "input";
  export type Conflict = "error" | "skip";
  export type Duration = `${number}${"ms" | "s" | "m" | "h" | "d"}`;
  export interface Context {
    readonly file: {
      readonly ref: FileRef;
      readonly name: string;
      readonly stem: string;
      readonly extension: string;
      readonly sizeBytes: number;
      readonly modifiedAt: string | null;
      readonly createdAt: string | null;
      readonly path: string | null;
    };
    readonly event: { readonly reason: "changed" | "reconcile" | "scheduled" | "manual" };
    readonly now: string;
  }
  export interface TransferOptions { to: string; onConflict?: Conflict }
  export interface RenameOptions { name: string; onConflict?: Conflict }
  export interface PathArgument { readonly pathOf: FileRef }
  export interface ExecOptions {
    program: string;
    args?: (string | PathArgument)[];
    cwd?: string;
    /** Complete explicit child environment; PATH is not inherited. */
    env?: Record<string, string>;
    timeout?: Duration;
  }
  export type Action =
    | { copy: TransferOptions & { file: FileRef } }
    | { move: TransferOptions & { file: FileRef } }
    | { rename: RenameOptions & { file: FileRef } }
    | { exec: ExecOptions };
  export const actions: {
    copy(file: FileRef, options: TransferOptions): Action;
    move(file: FileRef, options: TransferOptions): Action;
    rename(file: FileRef, options: RenameOptions): Action;
    exec(options: ExecOptions): Action;
    pathOf(file: FileRef): PathArgument;
  };
  export type Match = (ctx: Context) => boolean | Promise<boolean>;
  export type Actions = (ctx: Context) => Action[] | Promise<Action[]>;
  export type Run = (ctx: Context) => Action[] | null | undefined | Promise<Action[] | null | undefined>;
}
