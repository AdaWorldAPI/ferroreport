// Tests of the designer's model (site/designer/model.js) with the built engine:
// node bindings/wasm/tests/designer.mjs (after build.sh)
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import assert from "node:assert/strict";
import {
  fromWire, toWire, bands, detectFields, sampleRows, ensureField, insertOrdered, make, child, kids, blankReport,
  newElement, setMainText, readBorders, writeBorders, effectiveStyle, pageInfo, setPage, sampleFor, declaredFields,
} from "../../../site/designer/model.js";

const require = createRequire(import.meta.url);
const engine = require("../pkg");
const examples = new URL("../../../examples/", import.meta.url);
const read = (f) => readFileSync(new URL(f, examples), "utf8");

// Types of fields, from a sample of data.
const rows = sampleRows(JSON.parse(read("invoices.json")));
const fields = detectFields(rows);
assert.deepEqual(fields.map((f) => [f.name, f.type]), [["number", "Text"], ["customer", "Text"], ["overdue", "Yes/no"], ["lines", "List"]]);
assert.deepEqual(fields[3].fields.map((f) => [f.name, f.type, f.class]), [
  ["product", "Text", "java.lang.String"], ["qty", "Whole number", "java.lang.Integer"], ["price", "Number", "java.lang.Double"]]);
assert.deepEqual(detectFields([{ a: 1 }, { a: 2.5 }, { a: null, d: "2026-01-05" }]).map((f) => f.type), ["Number", "Date"]);
assert.equal(sampleRows({ rows: [{ a: 1 }] }).length, 1);
assert.equal(sampleRows({ a: 1, b: 2 }).length, 1);

// An example opened, changed and written back, by the engine.
const tree = fromWire(engine.toTree(read("invoices.ferro"), { resources: { "parts.ferro": read("parts.ferro") } }));
const list = bands(tree);
assert.deepEqual(list.map((b) => b.label), ["Title", "Page header", "Column header", "Detail", "Column footer", "Page footer", "Summary"]);
assert.equal(list.find((b) => b.kind === "detail").node.attrs.height, "62");
const footer = list.find((b) => b.key === "pageFooter");
assert.equal(footer.node, null);
const node = footer.create();
node.attrs.height = "20";
const el = newElement("textField", 0, 0, 100, 20, { bold: "true" });
setMainText(el, "$F{customer}");
node.children.push(el);
writeBorders(el, { width: 1, color: "#FF0000", style: "Solid", sides: { top: true, left: false, bottom: true, right: false } });
assert.deepEqual(readBorders(el).sides, { top: true, left: false, bottom: true, right: false });
ensureField(tree, "extra", "java.lang.String", true);
const jrxml = engine.fromTree(toWire(tree));
assert.ok(/<pageFooter height="20">\s*<element kind="textField" x="0" y="0" width="100" height="20" bold="true">/.test(jrxml), jrxml);
assert.ok(jrxml.includes('<topPen lineWidth="1" lineColor="#FF0000"/>'), jrxml);
// JRXML's order: the new field among the fields, the page footer after the detail.
assert.ok(jrxml.indexOf('name="extra"') > jrxml.indexOf('name="lines"') && jrxml.indexOf("<title") > jrxml.indexOf('name="extra"'));
assert.ok(jrxml.indexOf("<pageFooter") > jrxml.indexOf("</detail>"));
assert.equal(Buffer.from(engine.render(jrxml, rows).subarray(0, 5)).toString(), "%PDF-");
const ferro = engine.fromTree(toWire(tree), "ferro");
assert.ok(ferro.includes("pageFooter 20") && ferro.includes("$F{customer}"), ferro);

// Styles as an element shows them.
const title = child(tree, "title").children[0];
assert.equal(effectiveStyle(tree, title).fontSize, "16");
assert.equal(effectiveStyle(tree, el).fontName, "DejaVu Sans");

// Page settings keep the column between the margins.
setPage(tree, { width: 842, height: 595, margins: { left: 30, right: 30, top: 20, bottom: 20 } });
assert.deepEqual([pageInfo(tree).columnWidth, pageInfo(tree).landscape, tree.attrs.orientation], [782, true, "Landscape"]);

// A blank report renders, and made-up data covers a template's fields and lists.
const blank = blankReport("Blank");
assert.equal(Buffer.from(engine.render(engine.fromTree(toWire(blank)), []).subarray(0, 5)).toString(), "%PDF-");
const made = sampleFor(tree);
assert.equal(made.length, 3);
assert.ok(Array.isArray(made[0].lines) && made[0].lines.length === 2 && "price" in made[0].lines[0], JSON.stringify(made[0]));
assert.equal(Buffer.from(engine.render(jrxml, made).subarray(0, 5)).toString(), "%PDF-");

// insertOrdered puts parts where JRXML expects them.
const r = make("jasperReport", {}, [make("field", { name: "a" }), make("detail")]);
insertOrdered(r, make("title"));
insertOrdered(r, make("style", { name: "s" }));
assert.deepEqual(r.children.map((c) => c.tag), ["style", "field", "title", "detail"]);
assert.deepEqual(declaredFields(r).map((f) => f.name), ["a"]);
assert.equal(kids(r, "field").length, 1);

console.log("ok: designer model");
