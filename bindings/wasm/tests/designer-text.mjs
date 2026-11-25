// Tests of the designer's text without code (site/designer/model.js): expressions shown as
// `.ferro` texts (`Invoice {number}`) and back, checked against what the engine makes of the
// same `.ferro` text. node bindings/wasm/tests/designer-text.mjs (after build.sh)
import { createRequire } from "node:module";
import assert from "node:assert/strict";
import {
  expressionToText, textToExpression, parseText, staticToText, expressionSegments, javaString,
} from "../../../site/designer/model.js";

const require = createRequire(import.meta.url);
const engine = require("../pkg");

// Expressions the Text box shows as text, and the text it shows.
const simple = [
  ['$F{number} + " · " + $F{customer}', "{number} · {customer}"],
  ['"Invoice " + $F{number} + " · " + $F{customer}', "Invoice {number} · {customer}"],
  ['"Total: " + $F{amount}', "Total: {amount}"],
  ["$F{amount}", "{amount}"],
  ["  $F{amount}\n", "{amount}"],
  ['"" + $F{a} + $F{b}', "{a}{b}"],
  ['"Page " + $V{PAGE_NUMBER}', "Page $V{PAGE_NUMBER}"],
  ['"Dear " + $P{CUSTOMER} + ","', "Dear $P{CUSTOMER},"],
  ['$F{customer.name} + "!"', "{customer.name}!"],
  ['$F{first name} + " x"', "$F{first name} x"],
  ['"Hello"', "Hello"],
  ['""', ""],
  ['"Say \\"hi\\"" + $F{a}', 'Say "hi"{a}'],
  ['"Line 1\\nLine 2 " + $F{a}', "Line 1\nLine 2 {a}"],
  ['"{x} " + $F{a}', "\\{x} {a}"],
  ['"{0} " + $F{a}', "{0} {a}"],
  ['"{" + $F{a}', "\\{$F{a}"],
  ['$F{a} + "}"', "{a}\\}"],
  ['"C:\\\\" + $F{dir}', "C:\\\\{dir}"],
  ['"a\\\\b " + $F{a}', "a\\b {a}"],
  ['"costs $F{x} " + $F{a}', "costs \\$F\\{x} {a}"],
  ['"$" + $F{F}', "\\${F}"],
  ['"$F" + $F{a}', "\\$F{a}"],
  ['"=" + $F{a}', "={a}"],
  ['"Año " + $F{año}', "Año {año}"],
  ['"\\u00e9 " + $F{a}', "é {a}"],
];
for (const [expr, text] of simple) {
  assert.equal(expressionToText(expr), text, `shown: ${expr}`);
  // And back: the same expression (spaces aside), or one that prints the same.
  const back = textToExpression(text);
  if (text === "Hello" || text === "") assert.equal(back.static, JSON.parse(expr), `static: ${text}`);
  else assert.equal(expressionToText(back.expression), text, `round trip: ${text} -> ${back.expression}`);
}
assert.equal(textToExpression("Invoice {number} · {customer}").expression, '"Invoice " + $F{number} + " · " + $F{customer}');
assert.equal(textToExpression("{number} · {customer}").expression, '$F{number} + " · " + $F{customer}');
assert.equal(textToExpression("Total: {amount}").expression, '"Total: " + $F{amount}');
assert.equal(textToExpression("{amount}").expression, "$F{amount}");
assert.equal(textToExpression("{a}{b}").expression, '"" + $F{a} + $F{b}');
assert.equal(textToExpression('He said "{a}"\n').expression, '"He said \\"" + $F{a} + "\\"\\n"');
assert.deepEqual(textToExpression("Overdue: send a reminder"), { static: "Overdue: send a reminder" });
assert.deepEqual(textToExpression("Not {a hole}, {0} or {{x}}"), { static: "Not {a hole}, {0} or {{x}}" });
assert.deepEqual(textToExpression("\\{a\\} costs \\$5 \\\\"), { static: "{a} costs $5 \\" });

// Expressions that are not a text made of pieces: they stay formulas, untouched.
const formulas = [
  "$F{qty} * $F{price}",
  "$F{a} + $F{b}",
  "$F{a} + $F{b} + \" total\"",
  '"" + $F{a}',
  '$F{a} + ""',
  "$F{name}.toUpperCase()",
  '"Total: " + $F{amount}.toString()',
  '$F{overdue} ? "Overdue" : "Paid"',
  "new java.util.Date()",
  "new net.sf.jasperreports.engine.data.JRMapCollectionDataSource($F{lines})",
  '"a" + ($F{b} + $F{c})',
  '"a" - $F{b}',
  '"unterminated',
  "'c' + $F{a}",
  "$F{a} + + \"x\"",
  "",
  "   ",
  '"a" "b"',
  'msg("{0}", $F{a})',
  '"\\q" + $F{a}',
];
for (const expr of formulas) assert.equal(expressionToText(expr), null, `a formula: ${expr}`);

// Static texts with special characters show escaped, and come back the same.
for (const s of ["Overdue", "{name} is not a field here", "Price in $", "a\\b", "{{x}}", "}"]) {
  const shown = staticToText(s);
  assert.deepEqual(textToExpression(shown), { static: s }, `static round trip: ${s} as ${shown}`);
}
assert.equal(staticToText("Overdue"), "Overdue");
assert.equal(staticToText("{name} here"), "\\{name} here");

// The canvas: pieces of text and values, or a formula with its values picked out.
assert.deepEqual(expressionSegments('$F{number} + " · " + $F{customer}').segs,
  [{ ref: { kind: "F", name: "number" } }, { lit: " · " }, { ref: { kind: "F", name: "customer" } }]);
assert.deepEqual(expressionSegments("$F{qty} * $F{price}").formula,
  [{ ref: { kind: "F", name: "qty" } }, { lit: " * " }, { ref: { kind: "F", name: "price" } }]);
assert.deepEqual(parseText("Page $V{PAGE_NUMBER} of $V{PAGE_COUNT}").map((s) => s.ref ? s.ref.name : s.lit), ["Page ", "PAGE_NUMBER", " of ", "PAGE_COUNT"]);

// The engine reads the same texts in a .ferro `text` part as the designer does.
const texts = ["Invoice {number} · {customer}", "{amount}", "{a}{b}", "Total: {amount}", "\\{x} {a}", "\\{$F{a}",
  "{a}\\}", "C:\\\\{dir}", "costs \\$F\\{x} {a}", "\\${F}", "Dear $P{CUSTOMER},", "{customer.name}!", "Año {año}",
  "Overdue", "Not {a hole}, {0} or {{x}}", "\\{a\\} costs \\$5 \\\\"];
for (const text of texts) {
  const ferro = `report "T"\n  parameter CUSTOMER String\n  detail\n    text 0,0 200x20 :${JSON.stringify(text).replace(/^"|"$/g, "") === text ? ` ${text}` : `= ${JSON.stringify(text)}`}\n`;
  const tree = engine.toTree(ferro);
  const detail = tree.children.find((c) => c.tag === "detail");
  const el = detail.children[0].children[0];
  const kind = el.attrs.find(([k]) => k === "kind")[1];
  const mine = textToExpression(text);
  if (mine.static !== undefined) {
    assert.equal(kind, "staticText", `engine: ${text} is static`);
    assert.equal(el.children.find((c) => c.tag === "text").text, mine.static, `engine: static ${text}`);
  } else {
    assert.equal(kind, "textField", `engine: ${text} is a field`);
    assert.equal(el.children.find((c) => c.tag === "expression").text, mine.expression, `engine: ${text}`);
  }
}
assert.equal(javaString('a"b\\c\n'), '"a\\"b\\\\c\\n"');

console.log("ok: designer text");

// Every word the designer shows is in both languages: the keys app.js and index.html use.
const { DICTIONARY } = await import("../../../site/designer/i18n.js");
const { readFileSync } = await import("node:fs");
const site = new URL("../../../site/designer/", import.meta.url);
const app = readFileSync(new URL("app.js", site), "utf8");
const html = readFileSync(new URL("index.html", site), "utf8");
const used = new Set([
  ...[...app.matchAll(/\bt\("([^"]+)"/g)].map((m) => m[1]),
  ...[...html.matchAll(/data-i18n(?:-title|-aria|-ph)?="([^"]+)"/g)].map((m) => m[1]),
]);
for (const lang of ["en", "es"]) {
  const missing = [...used].filter((k) => !(k in DICTIONARY[lang]));
  assert.deepEqual(missing, [], `${lang} lacks ${missing.join(", ")}`);
  assert.deepEqual(Object.keys(DICTIONARY[lang]).sort(), Object.keys(DICTIONARY.en).sort(), `${lang} has the same words as en`);
}
for (const k of ["title", "pageHeader", "columnHeader", "detail", "columnFooter", "pageFooter", "lastPageFooter", "summary", "background", "noData", "groupHeader", "groupFooter"]) {
  assert.ok(`band.${k}` in DICTIONARY.es && `band.${k}.where` in DICTIONARY.es, `band ${k}`);
}
console.log("ok: designer words in English and Spanish");
