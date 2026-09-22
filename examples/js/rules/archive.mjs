/// <reference path="../../../types/api-v1.d.ts" />
import { actions as a } from "@app/api";
import { archiveName } from "./helpers.mjs";

export const apiVersion = 1;

/** @param {import('@app/api').Context} ctx */
export function match(ctx) {
  return ctx.file.extension === "pdf";
}

/** @param {import('@app/api').Context} ctx */
export function actions(ctx) {
  return [
    a.copy(ctx.file.ref, { to: "./backup" }),
    a.rename(ctx.file.ref, { name: archiveName(ctx.file.stem) }),
    a.move(ctx.file.ref, { to: "./archive" }),
  ];
}
