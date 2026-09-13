// Loads the IIFE bundle the way the engine injects it and checks that it
// evaluates without touching the DOM at load time and exposes the API.
// Behavioural tests run against real Chromium in `crates/engine-cdp`.
import { readFileSync } from "node:fs";
import vm from "node:vm";
import assert from "node:assert/strict";

const src = readFileSync(new URL("../dist/sensor.iife.js", import.meta.url), "utf8");
const ctx = vm.createContext({ performance, console, setTimeout });
vm.runInContext(src, ctx);

const api = ctx.BrowseSensor;
assert.ok(api, "BrowseSensor global missing");
for (const fn of ["snapshot", "watch", "resolveRef", "readMore", "installDefaultHost"]) {
  assert.equal(typeof api[fn], "function", `${fn} missing`);
}
assert.equal(api.readMore("nope"), null);
assert.equal(api.resolveRef(1), null);
assert.ok(!/__browseEmit\s*=[^=]/.test(src), "bundle must not define the host binding itself");
console.log("sensor bundle ok:", (src.length / 1024).toFixed(1), "KiB");
