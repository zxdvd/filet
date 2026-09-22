import * as rule from "filet:entry";

if ("apiVersion" in rule && rule.apiVersion !== 1) {
  throw new TypeError("apiVersion export must agree with manifest version 1");
}
const hasRun = "run" in rule;
const structured = "match" in rule || "actions" in rule;
if (hasRun === structured) {
  throw new TypeError("export either run, or match and actions");
}
if (hasRun ? typeof rule.run !== "function" :
  (typeof rule.match !== "function" || typeof rule.actions !== "function")) {
  throw new TypeError("rule entry points must be functions");
}
function freeze(value) {
  if (value && typeof value === "object") {
    for (const key of Object.keys(value)) freeze(value[key]);
    Object.freeze(value);
  }
  return value;
}
export async function evaluate(json) {
  const ctx = freeze(JSON.parse(json));
  let result;
  if (hasRun) {
    result = await rule.run(ctx);
    if (result == null) result = [];
  } else {
    const match = await rule.match(ctx);
    if (typeof match !== "boolean") throw new TypeError("match must return a boolean");
    if (!match) return "[]";
    result = await rule.actions(ctx);
  }
  if (!Array.isArray(result)) throw new TypeError("actions must return an array");
  if (result.length > 64) throw new RangeError("at most 64 actions");
  const output = JSON.stringify(result);
  if (output.length > 1048576) throw new RangeError("result exceeds 1 MiB");
  return output;
}
