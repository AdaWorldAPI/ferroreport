// The report being designed: the JRXML 7 tree that the engine's toTree() gives, with helpers
// to read and change it. No file format is read or written here: the engine does that.
// Attributes are kept as plain objects (in order) while editing, and as [name, value] pairs
// on the way in and out of the engine.

let nextId = 1;

/** A tree from the engine, ready to edit: attributes as objects, a hidden id on each node. */
export function fromWire(node) {
  const attrs = {};
  for (const [k, v] of node.attrs || []) attrs[k] = v;
  return {
    tag: node.tag,
    attrs,
    children: (node.children || []).map(fromWire),
    ...(node.text !== undefined ? { text: node.text } : {}),
    _id: nextId++,
  };
}

/** The tree the engine reads back. */
export function toWire(node) {
  const out = { tag: node.tag, attrs: Object.entries(node.attrs), children: node.children.map(toWire) };
  if (node.text !== undefined && node.children.length === 0) out.text = node.text;
  return out;
}

/** Gives fresh ids to a copied subtree. */
export function reId(node) {
  node._id = nextId++;
  node.children.forEach(reId);
  return node;
}

export const make = (tag, attrs = {}, children = [], text) => ({
  tag, attrs, children, ...(text !== undefined ? { text } : {}), _id: nextId++,
});

export const child = (n, tag) => n.children.find((c) => c.tag === tag);
export const kids = (n, tag) => n.children.filter((c) => c.tag === tag);
export const num = (n, k, d = 0) => {
  const v = parseFloat(n.attrs[k]);
  return Number.isFinite(v) ? v : d;
};
export function setAttr(n, k, v) {
  if (v === null || v === undefined || v === "" || v === false) delete n.attrs[k];
  else n.attrs[k] = typeof v === "number" ? String(round(v)) : String(v);
}
export const round = (v) => Math.round(v * 100) / 100;

// Where each part of a report goes among <jasperReport>'s children, as JasperReports 7 writes it.
const ROOT_ORDER = ["property", "import", "template", "style", "dataset", "parameter", "query", "field",
  "sortField", "variable", "filterExpression", "group", "background", "title", "pageHeader", "columnHeader",
  "detail", "columnFooter", "pageFooter", "lastPageFooter", "summary", "noData"];

/** Puts `node` among the children of `parent` where JRXML expects it. */
export function insertOrdered(parent, node, order = ROOT_ORDER) {
  const rank = order.indexOf(node.tag);
  let at = parent.children.length;
  for (let i = parent.children.length - 1; i >= 0; i--) {
    const r = order.indexOf(parent.children[i].tag);
    if (r !== -1 && r <= rank) { at = i + 1; break; }
    if (r > rank) at = i;
  }
  parent.children.splice(at, 0, node);
  return node;
}

// ---------------------------------------------------------------------------------------------
// Bands

export const BAND_LABELS = {
  background: "Background", title: "Title", pageHeader: "Page header", columnHeader: "Column header",
  detail: "Detail", columnFooter: "Column footer", pageFooter: "Page footer", lastPageFooter: "Last page footer",
  summary: "Summary", noData: "No data",
};
export const BAND_HELP = {
  background: "Behind every page", title: "Once, at the top of the first page", pageHeader: "At the top of every page",
  columnHeader: "Above the rows, on every page", detail: "Once for every row of data", columnFooter: "Below the rows, on every page",
  pageFooter: "At the bottom of every page", lastPageFooter: "At the bottom of the last page", summary: "Once, after the last row",
  noData: "When there is no data", groupHeader: "Before each group", groupFooter: "After each group",
};
// Bands shown even when the template does not have them yet, so there is somewhere to drop.
const STANDARD = ["title", "pageHeader", "columnHeader", "detail", "columnFooter", "pageFooter", "summary"];

/**
 * The report's bands from top to bottom: `{ key, kind, label, help, node, create }`. `node` is
 * the element holding the band's parts (with its `height`); a band the template does not have
 * yet has `node: null` and `create()` adds it.
 */
export function bands(root) {
  const out = [];
  const simple = (tag) => {
    const node = child(root, tag);
    if (node || STANDARD.includes(tag)) {
      out.push({
        key: tag, kind: tag, label: BAND_LABELS[tag], help: BAND_HELP[tag], node: node || null,
        create: () => node || insertOrdered(root, make(tag, { height: "0" })),
      });
    }
  };
  const groups = kids(root, "group");
  ["background", "title", "pageHeader", "columnHeader"].forEach(simple);
  for (const g of groups) {
    const gh = child(g, "groupHeader");
    (gh ? kids(gh, "band") : []).forEach((b, i, all) => out.push({
      key: `gh:${g.attrs.name}:${i}`, kind: "groupHeader", group: g.attrs.name,
      label: `Group header: ${g.attrs.name}${all.length > 1 ? ` (${i + 1})` : ""}`, help: BAND_HELP.groupHeader, node: b,
    }));
  }
  const detail = child(root, "detail");
  const detailBands = detail ? kids(detail, "band") : [];
  if (detailBands.length === 0) {
    out.push({
      key: "detail:0", kind: "detail", label: "Detail", help: BAND_HELP.detail, node: null,
      create: () => {
        const d = child(root, "detail") || insertOrdered(root, make("detail"));
        return kids(d, "band")[0] || (d.children.push(make("band", { height: "0" })), kids(d, "band")[0]);
      },
    });
  }
  detailBands.forEach((b, i) => out.push({
    key: `detail:${i}`, kind: "detail", label: detailBands.length > 1 ? `Detail ${i + 1}` : "Detail",
    help: BAND_HELP.detail, node: b,
  }));
  for (const g of [...groups].reverse()) {
    const gf = child(g, "groupFooter");
    (gf ? kids(gf, "band") : []).forEach((b, i, all) => out.push({
      key: `gf:${g.attrs.name}:${i}`, kind: "groupFooter", group: g.attrs.name,
      label: `Group footer: ${g.attrs.name}${all.length > 1 ? ` (${i + 1})` : ""}`, help: BAND_HELP.groupFooter, node: b,
    }));
  }
  ["columnFooter", "pageFooter", "lastPageFooter", "summary", "noData"].forEach(simple);
  return out;
}

/** The parts placed directly in a band or container (frames, a list's contents). */
export const elementsOf = (container) => container.children.filter((c) => c.tag === "element");

/** For an element that holds other elements (a frame, or a list), the node that holds them. */
export function innerContainer(el) {
  if (el.tag !== "element") return null;
  if (el.attrs.kind === "frame") return el;
  if (el.attrs.kind === "component") {
    const comp = child(el, "component");
    if (comp && comp.attrs.kind === "list") return child(comp, "contents") || null;
  }
  return null;
}

/** Index of every node: id -> { node, parent }. */
export function indexTree(root) {
  const map = new Map();
  const walk = (n, parent) => {
    map.set(n._id, { node: n, parent });
    n.children.forEach((c) => walk(c, n));
  };
  walk(root, null);
  return map;
}

/** For a list's contents, the dataset its fields come from. */
export function datasetOfContainer(root, container, index) {
  let n = container;
  while (n) {
    if (n.tag === "contents") {
      const comp = index.get(n._id).parent;
      const run = child(comp, "datasetRun");
      const name = run && run.attrs.subDataset;
      return kids(root, "dataset").find((d) => d.attrs.name === name) || null;
    }
    const e = index.get(n._id);
    n = e && e.parent;
  }
  return null;
}

// ---------------------------------------------------------------------------------------------
// Elements

export const KIND_LABELS = {
  staticText: "Text", textField: "Field", line: "Line", rectangle: "Rectangle", ellipse: "Ellipse",
  image: "Image", frame: "Frame", component: "Component", break: "Page break", subreport: "Subreport",
  chart: "Chart", crosstab: "Crosstab", genericElement: "Element",
};

export function elementLabel(el) {
  const k = el.attrs.kind;
  if (k === "component") {
    const comp = child(el, "component");
    const ck = comp && comp.attrs.kind;
    if (ck === "list") return "Repeating list";
    if (ck === "table") return "Table";
    return ck ? `${ck[0].toUpperCase()}${ck.slice(1)}` : "Component";
  }
  return KIND_LABELS[k] || k || "Element";
}

/** The text shown inside an element on the canvas. */
export function elementText(el) {
  const k = el.attrs.kind;
  if (k === "staticText") return (child(el, "text") || {}).text || "";
  if (k === "textField") return prettyExpression((child(el, "expression") || {}).text || "");
  return "";
}

/** `$F{name}` reads as "name" on the canvas; anything else as written. */
export function prettyExpression(e) {
  const m = /^\s*\$([FPV])\{([^}]+)\}\s*$/.exec(e);
  return m ? `${m[2]}` : e;
}

/** Sets the text of a static text, or the expression of a text field or image. */
export function setMainText(el, value) {
  const tag = el.attrs.kind === "staticText" ? "text" : "expression";
  let c = child(el, tag);
  if (!c) {
    c = make(tag, {}, [], "");
    el.children.unshift(c);
  }
  c.text = value;
}

export function newElement(kind, x, y, width, height, attrs = {}) {
  return make("element", { kind, x: String(round(x)), y: String(round(y)), width: String(round(width)), height: String(round(height)), ...attrs });
}

// Borders: <box><pen .../> for all sides, or <topPen/> ... for some.
const SIDES = ["top", "left", "bottom", "right"];

/** `{ width, color, style, sides: { top, left, bottom, right } }` from an element's box. */
export function readBorders(el) {
  const box = child(el, "box");
  const out = { width: 0, color: "#000000", style: "Solid", sides: { top: false, left: false, bottom: false, right: false } };
  if (!box) return out;
  const pen = child(box, "pen");
  for (const s of SIDES) {
    const p = child(box, `${s}Pen`) || pen;
    const w = p ? parseFloat(p.attrs.lineWidth) : NaN;
    if (Number.isFinite(w) && w > 0) {
      out.sides[s] = true;
      out.width = w;
      if (p.attrs.lineColor) out.color = p.attrs.lineColor;
      if (p.attrs.lineStyle) out.style = p.attrs.lineStyle;
    }
  }
  return out;
}

export function writeBorders(el, { width, color, style, sides }) {
  let box = child(el, "box");
  if (!box) {
    box = make("box");
    el.children.push(box);
  }
  box.children = box.children.filter((c) => c.tag !== "pen" && !c.tag.endsWith("Pen"));
  const on = SIDES.filter((s) => sides[s]);
  const pen = (tag) => {
    const a = { lineWidth: String(width) };
    if (color && color.toLowerCase() !== "#000000") a.lineColor = color;
    if (style && style !== "Solid") a.lineStyle = style;
    return make(tag, a);
  };
  if (width > 0 && on.length === 4) box.children.push(pen("pen"));
  else if (width > 0) on.forEach((s) => box.children.push(pen(`${s}Pen`)));
  if (box.children.length === 0 && Object.keys(box.attrs).length === 0) el.children.splice(el.children.indexOf(box), 1);
}

/** The stroke of a line, rectangle or ellipse: <pen lineWidth lineStyle/>. */
export function readPen(el) {
  const p = child(el, "pen");
  return { width: p ? num(p, "lineWidth", 1) : 1, style: (p && p.attrs.lineStyle) || "Solid" };
}
export function writePen(el, { width, style }) {
  let p = child(el, "pen");
  if (!p) {
    p = make("pen");
    el.children.push(p);
  }
  setAttr(p, "lineWidth", width === 1 ? null : width);
  setAttr(p, "lineStyle", style === "Solid" ? null : style);
  if (Object.keys(p.attrs).length === 0) el.children.splice(el.children.indexOf(p), 1);
}

// ---------------------------------------------------------------------------------------------
// Styles: what an element looks like once its style (and the default style) apply.

const STYLE_KEYS = ["fontName", "fontSize", "bold", "italic", "underline", "strikeThrough", "forecolor", "backcolor",
  "mode", "hTextAlign", "vTextAlign"];

export function effectiveStyle(root, el) {
  const styles = new Map(kids(root, "style").map((s) => [s.attrs.name, s]));
  const def = kids(root, "style").find((s) => s.attrs.default === "true");
  const out = {};
  const apply = (s, depth = 0) => {
    if (!s || depth > 10) return;
    if (s.attrs.style) apply(styles.get(s.attrs.style), depth + 1);
    for (const k of STYLE_KEYS) if (s.attrs[k] !== undefined) out[k] = s.attrs[k];
  };
  if (el.attrs.style) apply(styles.get(el.attrs.style));
  else apply(def);
  for (const k of STYLE_KEYS) if (el.attrs[k] !== undefined) out[k] = el.attrs[k];
  return out;
}

// ---------------------------------------------------------------------------------------------
// Fields

export const TYPE_CLASSES = {
  Text: "java.lang.String", "Whole number": "java.lang.Integer", Number: "java.lang.Double",
  "Yes/no": "java.lang.Boolean", Date: "java.util.Date", List: "java.util.List", Object: "java.util.Map",
};
export const CLASS_TYPES = Object.fromEntries(Object.entries(TYPE_CLASSES).map(([k, v]) => [v, k]));
Object.assign(CLASS_TYPES, {
  "java.lang.Long": "Whole number", "java.lang.Short": "Whole number", "java.math.BigDecimal": "Number",
  "java.lang.Float": "Number", "java.sql.Timestamp": "Date", "java.sql.Date": "Date", "java.time.LocalDate": "Date",
  "java.util.Collection": "List",
});

const DATE_RE = /^\d{4}-\d{2}-\d{2}([T ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})?)?$/;

function typeOf(v) {
  if (v === null || v === undefined) return null;
  if (typeof v === "boolean") return "Yes/no";
  if (typeof v === "number") return Number.isInteger(v) ? "Whole number" : "Number";
  if (typeof v === "string") return DATE_RE.test(v) ? "Date" : "Text";
  if (Array.isArray(v)) return "List";
  return "Object";
}

/** The rows of a sample: an array of objects, one object, or an object holding one array. */
export function sampleRows(sample) {
  if (Array.isArray(sample)) return sample.filter((r) => r && typeof r === "object" && !Array.isArray(r));
  if (sample && typeof sample === "object") {
    const arrays = Object.values(sample).filter(Array.isArray);
    if (arrays.length === 1 && Object.keys(sample).length === 1) return sampleRows(arrays[0]);
    return [sample];
  }
  return [];
}

/**
 * The fields in rows of data: `[{ name, type, class, fields? }]`, where `type` is a friendly
 * name ("Text", "Number"...) and a list of objects has the fields of its items.
 */
export function detectFields(rows) {
  const seen = new Map();
  for (const row of rows.slice(0, 500)) {
    for (const [k, v] of Object.entries(row)) {
      const t = typeOf(v);
      const prev = seen.get(k);
      let type = prev ? prev.type : null;
      if (t && !type) type = t;
      else if (t && type !== t) {
        if ((type === "Whole number" && t === "Number") || (type === "Number" && t === "Whole number")) type = "Number";
        else if (type === "Date" && t === "Text") type = "Text";
        else if (type === "Text" && t === "Date") type = "Text";
        else type = type || t;
      }
      const entry = prev || { name: k, type: null, items: [] };
      entry.type = type;
      if (Array.isArray(v)) entry.items.push(...v.filter((x) => x && typeof x === "object" && !Array.isArray(x)));
      seen.set(k, entry);
    }
  }
  return [...seen.values()].map((e) => {
    const type = e.type || "Text";
    const out = { name: e.name, type, class: TYPE_CLASSES[type] };
    if (type === "List") out.fields = detectFields(e.items);
    return out;
  });
}

/** The fields a template (or one of its datasets) declares: `[{ name, class, type }]`. */
export function declaredFields(holder) {
  return kids(holder, "field").map((f) => ({ name: f.attrs.name, class: f.attrs.class || "java.lang.String", type: CLASS_TYPES[f.attrs.class] || "Text" }));
}

/** Declares a field in the template or a dataset, unless it is there already. */
export function ensureField(holder, name, cls, isRoot) {
  if (kids(holder, "field").some((f) => f.attrs.name === name)) return false;
  const f = make("field", { name, class: cls || "java.lang.String" });
  if (isRoot) insertOrdered(holder, f);
  else {
    // In a dataset, fields go after its parameters and query, before its variables.
    const order = ["property", "parameter", "query", "field", "sortField", "variable", "filterExpression", "group"];
    insertOrdered(holder, f, order);
  }
  return true;
}

/** A template name that is safe to use as a JRXML name. */
export const safeName = (s) => (s || "").replace(/[^\p{L}\p{N}_ -]/gu, "").trim() || "Report";

// ---------------------------------------------------------------------------------------------
// Pages

export const PAGE_SIZES = {
  A4: [595, 842], Letter: [612, 792], Legal: [612, 1008], A3: [842, 1191], A5: [420, 595],
};

export function pageInfo(root) {
  const a = root.attrs;
  const w = num(root, "pageWidth", 595), h = num(root, "pageHeight", 842);
  const m = { left: num(root, "leftMargin", 20), right: num(root, "rightMargin", 20), top: num(root, "topMargin", 20), bottom: num(root, "bottomMargin", 20) };
  const landscape = a.orientation === "Landscape" || w > h;
  let size = "Custom";
  for (const [name, [pw, ph]] of Object.entries(PAGE_SIZES)) {
    if ((pw === w && ph === h) || (pw === h && ph === w)) size = name;
  }
  const cols = num(root, "columnCount", 1);
  const columnWidth = num(root, "columnWidth", w - m.left - m.right);
  return { width: w, height: h, margins: m, landscape, size, columns: cols, columnWidth };
}

/** Changes the page and keeps the column as wide as what is between the margins. */
export function setPage(root, { width, height, margins }) {
  setAttr(root, "pageWidth", width);
  setAttr(root, "pageHeight", height);
  if (width > height) root.attrs.orientation = "Landscape";
  else delete root.attrs.orientation;
  for (const [k, v] of Object.entries(margins)) root.attrs[`${k}Margin`] = String(round(v));
  const cols = num(root, "columnCount", 1);
  const spacing = num(root, "columnSpacing", 0);
  setAttr(root, "columnWidth", Math.max(1, Math.floor((width - margins.left - margins.right - spacing * (cols - 1)) / cols)));
}

/** A blank report: A4, a default style, and the usual bands. */
export function blankReport(name = "Report") {
  return make("jasperReport", {
    name, pageWidth: "595", pageHeight: "842", columnWidth: "555", leftMargin: "20", rightMargin: "20", topMargin: "20", bottomMargin: "20",
  }, [
    make("style", { name: "Base", default: "true", fontName: "DejaVu Sans", fontSize: "10" }),
    make("title", { height: "40" }),
    make("columnHeader", { height: "20" }),
    make("detail", {}, [make("band", { height: "20" })]),
    make("pageFooter", { height: "20" }),
  ]);
}

/**
 * Made-up rows for a template's fields, for when there is no data to preview it with: three
 * rows of plausible values, and a few items in each list.
 */
export function sampleFor(root) {
  const lists = new Map();
  const walk = (n) => {
    if (n.tag === "datasetRun") {
      const src = child(n, "dataSourceExpression");
      const m = src && /\$F\{([^}]+)\}/.exec(src.text || "");
      const ds = kids(root, "dataset").find((d) => d.attrs.name === n.attrs.subDataset);
      if (m && ds) lists.set(m[1], ds);
    }
    n.children.forEach(walk);
  };
  walk(root);
  const value = (f, i) => {
    switch (CLASS_TYPES[f.class] || "Text") {
      case "Number": return round(120.5 * (i + 1) + 0.25);
      case "Whole number": return i + 1;
      case "Yes/no": return i % 2 === 0;
      case "Date": return `2026-0${(i % 9) + 1}-1${i % 10}`;
      case "List": return [];
      case "Object": return {};
      default: return `${f.name} ${i + 1}`;
    }
  };
  const rows = (fields, n, depth = 0) => Array.from({ length: n }, (_, i) => Object.fromEntries(fields.map((f) => {
    if ((CLASS_TYPES[f.class] || "") === "List" && lists.has(f.name) && depth < 2) return [f.name, rows(declaredFields(lists.get(f.name)), 2, depth + 1)];
    return [f.name, value(f, i)];
  })));
  return rows(declaredFields(root), 3);
}

// ---------------------------------------------------------------------------------------------
// Text without code: an expression made of texts and values joined with + is shown and edited
// as `.ferro` 1.1 writes a `text`: `"Invoice " + $F{number}` is `Invoice {number}`. `$P{}`,
// `$V{}` and `$R{}` stay as they are written; `\{`, `\}`, `\$` and `\\` are the characters
// themselves. Anything else (a method call, arithmetic, a condition) is a formula, and is left
// as it is written.

// A name in `{name}`: letters, digits and `_`, not starting with a digit, in parts joined by `.`
// (as template.rs `field_ref` reads it).
const NAME_START = /[\p{Alphabetic}_]/u;
const NAME_CHAR = /[\p{Alphabetic}\p{N}_]/u;
const HOLE_AHEAD = /^\{[\p{Alphabetic}_][\p{Alphabetic}\p{N}_]*(?:\.[\p{Alphabetic}_][\p{Alphabetic}\p{N}_]*)*\}(?!\})/u;
const SHORT_NAME = /^[\p{Alphabetic}_][\p{Alphabetic}\p{N}_]*(?:\.[\p{Alphabetic}_][\p{Alphabetic}\p{N}_]*)*$/u;

/** Where `{name}` that starts at `i` ends (the index of its `}`), or -1. */
function fieldRef(cs, i) {
  let j = i + 1;
  for (;;) {
    if (!(j < cs.length && NAME_START.test(cs[j]))) return -1;
    while (j < cs.length && NAME_CHAR.test(cs[j])) j++;
    if (cs[j] === ".") j++;
    else if (cs[j] === "}" && cs[j + 1] !== "}") return j;
    else return -1;
  }
}

/**
 * A text as the designer's Text box (and `.ferro`) reads it: `[{ lit }, { ref: { kind, name } }]`,
 * where `kind` is F, P, V or R. Adjacent characters are one `lit`.
 */
export function parseText(text) {
  const cs = [...(text || "")];
  const segs = [];
  let lit = "";
  let i = 0;
  while (i < cs.length) {
    const c = cs[i];
    if (c === "\\" && i + 1 < cs.length && "{}$=\\".includes(cs[i + 1])) {
      lit += cs[i + 1];
      i += 2;
      continue;
    }
    let ref = null;
    if (c === "$" && i + 2 < cs.length && "FPVR".includes(cs[i + 1]) && cs[i + 2] === "{") {
      let e = -1;
      for (let j = i + 3; j < cs.length; j++) if (cs[j] === "}" || cs[j] === "{" || cs[j] === "\n") { e = j; break; }
      if (e > i + 3 && cs[e] === "}") ref = { kind: cs[i + 1], name: cs.slice(i + 3, e).join(""), next: e + 1 };
    } else if (c === "{" && (i === 0 || cs[i - 1] !== "{")) {
      const e = fieldRef(cs, i);
      if (e !== -1) ref = { kind: "F", name: cs.slice(i + 1, e).join(""), next: e + 1 };
    }
    if (ref) {
      if (lit) segs.push({ lit });
      lit = "";
      segs.push({ ref: { kind: ref.kind, name: ref.name } });
      i = ref.next;
    } else {
      lit += c;
      i += 1;
    }
  }
  if (lit) segs.push({ lit });
  return segs;
}

/** A Java string literal, as template.rs `java_string` writes it. */
export function javaString(s) {
  return `"${s.replace(/[\\"\n\t\r]/g, (c) => ({ "\\": "\\\\", '"': '\\"', "\n": "\\n", "\t": "\\t", "\r": "\\r" })[c])}"`;
}

const refText = ({ kind, name }) => `$${kind}{${name}}`;

/**
 * What the Text box's text makes: `{ static: "..." }` when it has no values in it, or
 * `{ expression: "..." }`, the expression `.ferro` makes of it.
 */
export function textToExpression(text) {
  const segs = parseText(text);
  if (!segs.some((s) => s.ref)) return { static: segs.map((s) => s.lit).join("") };
  const parts = segs.map((s) => (s.ref ? refText(s.ref) : javaString(s.lit)));
  // `{a}{b}` joins two texts, not two numbers.
  if (segs.length > 1 && segs[0].ref && segs[1].ref) parts.unshift('""');
  return { expression: parts.join(" + ") };
}

/** The parts of an expression that is only texts and values joined with +, or null. */
function exprTerms(expr) {
  const s = expr || "";
  let i = 0;
  const terms = [];
  const ws = () => { while (i < s.length && /\s/.test(s[i])) i++; };
  ws();
  if (i >= s.length) return null;
  for (;;) {
    ws();
    if (s[i] === '"') {
      let lit = "";
      i++;
      for (;;) {
        if (i >= s.length || s[i] === "\n") return null;
        const c = s[i];
        if (c === '"') { i++; break; }
        if (c === "\\") {
          const n = s[i + 1];
          const simple = { '"': '"', "\\": "\\", n: "\n", t: "\t", r: "\r", "'": "'", b: "\b", f: "\f" };
          if (n in simple) { lit += simple[n]; i += 2; continue; }
          const u = /^u+([0-9a-fA-F]{4})/.exec(s.slice(i + 1));
          if (u) { lit += String.fromCharCode(parseInt(u[1], 16)); i += 1 + u[0].length; continue; }
          return null;
        }
        lit += c;
        i++;
      }
      terms.push({ lit });
    } else {
      const m = /^\$([FPVR])\{([^{}\n]+)\}/.exec(s.slice(i));
      if (!m) return null;
      terms.push({ ref: { kind: m[1], name: m[2] } });
      i += m[0].length;
    }
    ws();
    if (i >= s.length) break;
    if (s[i] !== "+") return null;
    i++;
  }
  return terms;
}

/**
 * What an expression of texts and values prints, in a form two expressions can be compared by:
 * a value alone (which a number format applies to), or a text made of pieces. Null when it is not
 * text: `$F{a} + $F{b}` adds two numbers.
 */
function meaning(terms) {
  if (terms.length === 1 && terms[0].ref) return { value: terms[0].ref };
  if (terms.length > 1 && !("lit" in terms[0]) && !("lit" in terms[1])) return null;
  const pieces = [];
  for (const t of terms) {
    if (t.ref) pieces.push({ ref: t.ref });
    else if (t.lit && pieces.length && "lit" in pieces[pieces.length - 1]) pieces[pieces.length - 1].lit += t.lit;
    else if (t.lit) pieces.push({ lit: t.lit });
  }
  return { text: pieces };
}

/** The Text box's text for pieces of text and values; `full` escapes every special character. */
function piecesToText(pieces, full) {
  let out = "";
  pieces.forEach((p, idx) => {
    const prev = pieces[idx - 1];
    const next = pieces[idx + 1];
    if (p.ref) {
      const short = p.ref.kind === "F" && SHORT_NAME.test(p.ref.name) && !out.endsWith("{");
      out += short ? `{${p.ref.name}}` : refText(p.ref);
      return;
    }
    const cs = [...p.lit];
    cs.forEach((c, k) => {
      const rest = cs.slice(k).join("");
      const last = k === cs.length - 1;
      let esc = full;
      if (c === "\\") esc = esc || (last ? !!next : "{}$=\\".includes(cs[k + 1]));
      else if (c === "{") esc = esc || HOLE_AHEAD.test(rest) || (last && next && next.ref && next.ref.kind === "F");
      else if (c === "}") esc = esc || (k === 0 && prev && prev.ref);
      else if (c === "$") esc = esc || /^\$[FPVR]\{/.test(rest) || (cs.length - k <= 2 && /^\$[FPVR]?$/.test(rest) && !!next);
      else esc = false;
      out += esc ? `\\${c}` : c;
    });
  });
  return out;
}

const sameMeaning = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/**
 * The Text box's text for an expression, when it is only texts and values joined with +
 * (`"Invoice " + $F{number}` is `Invoice {number}`, `$F{total}` is `{total}`), or null when it is
 * a formula that the Text box cannot show as it is.
 */
export function expressionToText(expr) {
  const terms = exprTerms(expr);
  const m = terms && meaning(terms);
  if (!m) return null;
  const pieces = m.value ? [{ ref: m.value }] : m.text;
  for (const full of [false, true]) {
    const text = piecesToText(pieces, full);
    const back = textToExpression(text);
    const again = back.expression !== undefined ? exprTerms(back.expression) : [{ lit: back.static }];
    if (again && sameMeaning(meaning(again), m)) return text;
  }
  return null;
}

/** The Text box's text for a static text: the text with its special characters escaped. */
export function staticToText(s) {
  return expressionToText(javaString(s || "")) ?? s;
}

/**
 * How the canvas shows a text field: `{ segs }` (pieces of text and values) for one the Text
 * box can show, or `{ formula: [...] }`, the formula with its values picked out.
 */
export function expressionSegments(expr) {
  const text = expressionToText(expr);
  if (text !== null) return { segs: parseText(text) };
  const formula = [];
  for (const p of (expr || "").split(/(\$[FPVR]\{[^{}\n]+\})/)) {
    const m = /^\$([FPVR])\{(.+)\}$/.exec(p);
    if (m) formula.push({ ref: { kind: m[1], name: m[2] } });
    else if (p) formula.push({ lit: p });
  }
  return { formula };
}
