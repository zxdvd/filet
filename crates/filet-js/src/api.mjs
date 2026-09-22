/** API v1: pure action constructors; no filesystem or process access. */
export const actions = Object.freeze({
  copy(file, options) { return { copy: { ...options, file } }; },
  move(file, options) { return { move: { ...options, file } }; },
  rename(file, options) { return { rename: { ...options, file } }; },
  exec(options) { return { exec: { ...options } }; },
  pathOf(file) { return Object.freeze({ pathOf: file }); },
});
