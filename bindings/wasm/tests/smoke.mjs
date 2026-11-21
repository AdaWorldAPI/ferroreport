// Smoke test of the built npm package (run after build.sh): node tests/smoke.mjs
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import assert from "node:assert/strict";

const require = createRequire(import.meta.url);
const ferroreport = require("../pkg");
const fx = new URL("../../../fixtures/17-subreports/", import.meta.url);

const template = readFileSync(new URL("report.jrxml", fx), "utf8");
const data = JSON.parse(readFileSync(new URL("data.json", fx), "utf8"));
const pdf = ferroreport.render(template, data, {}, {
  resources: { "lines.jrxml": readFileSync(new URL("lines.jrxml", fx)) },
  now: "2026-01-01T00:00:00",
});
assert.ok(pdf instanceof Uint8Array);
assert.equal(Buffer.from(pdf.subarray(0, 5)).toString(), "%PDF-");

const info = ferroreport.inspect(template);
assert.equal(info.name, "orders");
assert.deepEqual(info.fields, ["order", "customer", "lines"]);

assert.throws(() => ferroreport.render("<jasperReport", [], {}, {}));
console.log(`ok: ${pdf.length} bytes, engine ${ferroreport.version()}`);
