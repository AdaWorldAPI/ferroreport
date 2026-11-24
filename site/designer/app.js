// The report designer: a page with the report's bands, parts placed on them, a properties
// panel, and a preview made by the ferroreport engine (WebAssembly) in the browser.
// Templates are read and written by the engine (toTree / fromTree); this file only edits the
// tree in between. Every word the page shows comes from i18n.js.
import {
  fromWire, toWire, reId, make, child, kids, num, setAttr, insertOrdered, bands, elementsOf, innerContainer,
  indexTree, datasetOfContainer, setMainText, newElement, readBorders, writeBorders, readPen, writePen,
  effectiveStyle, detectFields, sampleRows, declaredFields, ensureField, safeName, PAGE_SIZES, pageInfo, setPage,
  blankReport, sampleFor, CLASS_TYPES, TYPE_CLASSES, BAND_LABELS, expressionToText, textToExpression, staticToText,
  expressionSegments, javaString, parseText,
} from "./model.js";
import { t, lang, setLang, applyStatic } from "./i18n.js";

const $ = (id) => document.getElementById(id);
const PT = 4 / 3; // CSS pixels in a point: at 100% the page is its real size
// Room for the names of the bands, left of the page (less on smaller screens).
const gutter = () => (window.innerWidth <= 1440 ? 124 : 156);
const EMPTY_BAND = 24; // how tall (in screen pixels) an empty band is drawn: a slim strip to drop on
const STORE = "ferroreport.designer.v1";
const WELCOMED = "ferroreport.designer.welcomed";
const HEADINGS = "ferroreport.designer.headings";
const LOCALES = ["en_US", "en_GB", "es_DO", "es_MX", "es_ES", "fr_FR", "de_DE", "pt_BR"];
const PATTERNS = ["", "#,##0.00", "#,##0", "$#,##0.00", "0%", "MMM d, yyyy", "dd/MM/yyyy", "yyyy-MM-dd", "MMMM d, yyyy HH:mm"];
const TODAY = "new java.util.Date()";

/** localStorage, which a browser may refuse (private mode, full, blocked). */
const store = {
  get(k) { try { return localStorage.getItem(k); } catch { return null; } },
  set(k, v) { try { localStorage.setItem(k, v); } catch { /* the page still works */ } },
};

/** Builds a DOM element: h("div", { class: "x", onclick }, child, "text"). */
function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === undefined || v === null || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "style" && typeof v === "object") Object.assign(el.style, v);
    else if (k === "dataset") Object.assign(el.dataset, v);
    else if (k in el && k !== "list" && typeof v !== "string") el[k] = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) if (c !== null && c !== undefined && c !== false) el.append(c instanceof Node ? c : String(c));
  return el;
}

// Small line icons, drawn in the text color.
const ICONS = {
  copy: '<rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M3.5 10.5h-.5a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1h6.5a1 1 0 0 1 1 1v.5"/>',
  trash: '<path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5"/>',
  front: '<rect x="2.5" y="2.5" width="7" height="7" rx="1"/><rect x="6.5" y="6.5" width="7" height="7" rx="1" fill="currentColor" fill-opacity=".18"/>',
  back: '<rect x="6.5" y="6.5" width="7" height="7" rx="1"/><rect x="2.5" y="2.5" width="7" height="7" rx="1" fill="currentColor" fill-opacity=".18"/>',
};
// Alignment: lines of text, and a block in a box.
const ALIGN = {
  Left: '<path d="M2.5 3.5h11M2.5 6.5h7M2.5 9.5h11M2.5 12.5h7"/>',
  Center: '<path d="M2.5 3.5h11M4.5 6.5h7M2.5 9.5h11M4.5 12.5h7"/>',
  Right: '<path d="M2.5 3.5h11M6.5 6.5h7M2.5 9.5h11M6.5 12.5h7"/>',
  Justified: '<path d="M2.5 3.5h11M2.5 6.5h11M2.5 9.5h11M2.5 12.5h11"/>',
  Top: '<path d="M2.5 2.5h11"/><rect x="5" y="4.5" width="6" height="5" rx="1"/>',
  Middle: '<path d="M2.5 8h2M11.5 8h2"/><rect x="5" y="5.5" width="6" height="5" rx="1"/>',
  Bottom: '<path d="M2.5 13.5h11"/><rect x="5" y="6.5" width="6" height="5" rx="1"/>',
};
Object.assign(ICONS, ALIGN);
const icon = (name) => {
  const s = h("span", { class: "ico-svg", "aria-hidden": "true" });
  s.innerHTML = `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">${ICONS[name]}</svg>`;
  return s;
};

// ---------------------------------------------------------------------------------------------
// State

const S = {
  root: blankReport(),
  name: "report",
  dataText: "[]",
  rows: [],
  paramsText: "{}",
  params: {},
  resources: {}, // files opened with the template: images, subreports (not kept across visits)
  dataFields: [],
  sel: null, // { type: "el", id } or { type: "band", key }
  zoom: 1,
  grid: 5,
  snap: true,
  locale: "en_US",
  undo: [],
  redo: [],
  coalesce: null,
  view: "design", // or "preview"
  open: new Set(), // sections of the properties panel that are open
};
let engine = null;
let index = new Map();
let bandList = [];
let dragPayload = null;
let previewUrl = null;
let previewShown = null;
let dataError = null;

const snapshot = () => JSON.stringify({ root: S.root, dataText: S.dataText });
function restore(snap) {
  const s = JSON.parse(snap);
  S.root = s.root;
  if (s.dataText !== S.dataText) setData(s.dataText, false);
}

/**
 * Changes the report: `fn` edits S.root, then the canvas, the panel and the preview follow.
 * Changes with the same `coalesce` key in a row are one step of undo (typing in a box).
 */
function change(fn, { coalesce = null, props = true } = {}) {
  const before = snapshot();
  const result = fn();
  if (result === false) return;
  const now = Date.now();
  if (!(coalesce && S.coalesce && S.coalesce.key === coalesce && now - S.coalesce.at < 1500)) {
    S.undo.push(before);
    if (S.undo.length > 200) S.undo.shift();
  }
  S.coalesce = coalesce ? { key: coalesce, at: now } : null;
  S.redo = [];
  refresh({ props });
}

function undo() {
  if (!S.undo.length) return;
  S.redo.push(snapshot());
  restore(S.undo.pop());
  S.coalesce = null;
  if (S.sel && S.sel.type === "el" && !indexTree(S.root).has(S.sel.id)) S.sel = null;
  refresh();
  announce(t("msg.undone"));
}
function redo() {
  if (!S.redo.length) return;
  S.undo.push(snapshot());
  restore(S.redo.pop());
  S.coalesce = null;
  if (S.sel && S.sel.type === "el" && !indexTree(S.root).has(S.sel.id)) S.sel = null;
  refresh();
  announce(t("msg.redone"));
}

function refresh({ props = true } = {}) {
  renderCanvas();
  if (props) renderProps();
  renderFields();
  $("undo").disabled = !S.undo.length;
  $("redo").disabled = !S.redo.length;
  schedulePreview();
  scheduleSave();
}

let toastTimer;
function announce(text, kind = "") {
  $("announce").textContent = text;
  let el = document.querySelector(".toast");
  if (!el) {
    el = h("div", { class: "toast", "aria-hidden": "true" });
    document.body.append(el);
  }
  el.textContent = text;
  el.className = `toast show ${kind}`;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove("show"), kind === "bad" ? 6000 : 2600);
}

// ---------------------------------------------------------------------------------------------
// Names people understand

/** The name of a band, what it is for, and its JasperReports name. */
function bandName(b) {
  const group = b.group || "";
  return {
    name: t(`band.${b.kind}`, { group }),
    where: t(`band.${b.kind}.where`, { group }),
    tech: b.kind === "groupHeader" || b.kind === "groupFooter" ? `${b.kind === "groupHeader" ? "Group header" : "Group footer"} ${group}` : BAND_LABELS[b.kind] || b.kind,
  };
}

/** What a part is called: Text, Field, Line, Box... */
function partName(el) {
  const k = el.attrs.kind;
  if (k === "component") {
    const comp = child(el, "component");
    const ck = comp && comp.attrs.kind;
    if (ck === "list") return t("kind.list");
    if (ck === "table") return t("kind.table");
    return ck ? `${ck[0].toUpperCase()}${ck.slice(1)}` : t("kind.component");
  }
  return t(`kind.${k}`, {}, k || "Element");
}

const typeName = (type) => t(`type.${type}`, {}, type);

/** How a value in a text reads: a field's name, "page number"... */
function refLabel({ kind, name }) {
  if (kind === "V") {
    const known = { PAGE_NUMBER: "pill.page", PAGE_COUNT: "pill.pages", REPORT_COUNT: "pill.row" }[name];
    if (known) return t(known);
  }
  return name;
}

/** What a text part says, in words, for screen readers. */
function spokenText(el) {
  const k = el.attrs.kind;
  if (k === "staticText") return (child(el, "text") || {}).text || "";
  if (k !== "textField") return "";
  const expr = (child(el, "expression") || {}).text || "";
  if (expr.trim() === TODAY) return t("pill.today");
  const shown = expressionSegments(expr);
  if (shown.segs) return shown.segs.map((s) => (s.ref ? `[${refLabel(s.ref)}]` : s.lit)).join("");
  return t("canvas.formula");
}

// ---------------------------------------------------------------------------------------------
// Geometry helpers

const scale = () => S.zoom * PT;
const snapV = (v) => (S.snap ? Math.round(v / S.grid) * S.grid : Math.round(v));
const geo = (el) => ({ x: num(el, "x"), y: num(el, "y"), w: num(el, "width"), h: num(el, "height") });
function setGeo(el, { x, y, w, h: hh }) {
  if (x !== undefined) el.attrs.x = String(Math.round(x));
  if (y !== undefined) el.attrs.y = String(Math.round(y));
  if (w !== undefined) el.attrs.width = String(Math.max(1, Math.round(w)));
  if (hh !== undefined) el.attrs.height = String(Math.max(el.attrs.kind === "line" ? 0 : 1, Math.round(hh)));
}

/** Width of a band, frame or list contents. */
function containerWidth(c) {
  if (c.tag === "element" || c.tag === "contents") return num(c, "width");
  return pageInfo(S.root).columnWidth;
}
const isBandNode = (n) => n && n.tag !== "element" && n.tag !== "contents";

/** Keeps a part inside its container, and makes the container (and those around it) tall enough. */
function fit(el, container) {
  const g = geo(el);
  const cw = containerWidth(container);
  if (g.w > cw) g.w = cw;
  g.x = Math.min(Math.max(0, g.x), Math.max(0, cw - g.w));
  g.y = Math.max(0, g.y);
  setGeo(el, g);
  grow(container, g.y + g.h);
}
function grow(container, bottom) {
  let c = container;
  let need = bottom;
  while (c) {
    if (num(c, "height") < need) c.attrs.height = String(Math.ceil(need));
    if (isBandNode(c)) break;
    const entry = index.get(c._id);
    if (!entry) break;
    // contents grow their list element; a frame grows its own container.
    if (c.tag === "contents") {
      const listEl = index.get(entry.parent._id).parent;
      if (num(listEl, "height") < num(c, "height")) listEl.attrs.height = c.attrs.height;
      need = num(listEl, "y") + num(listEl, "height");
      c = index.get(listEl._id).parent;
    } else {
      need = num(c, "y") + num(c, "height");
      c = entry.parent;
    }
  }
}

/** A free place for a part of w × h in a container: the first gap, or under everything. */
function freeSpot(container, w, hh) {
  const cw = containerWidth(container);
  const others = elementsOf(container).map(geo);
  const ch = isBandNode(container) || container.tag === "contents" || container.tag === "element" ? num(container, "height") : 0;
  const step = Math.max(S.grid, 5);
  for (let y = 0; y + hh <= ch; y += step) {
    for (let x = 0; x + w <= cw; x += step) {
      if (!others.some((o) => x < o.x + o.w && x + w > o.x && y < o.y + o.h && y + hh > o.y)) return { x, y };
    }
  }
  const bottom = others.reduce((m, o) => Math.max(m, o.y + o.h), 0);
  return { x: 0, y: bottom };
}

// ---------------------------------------------------------------------------------------------
// Canvas

/** A band with nothing on it is drawn as a slim strip, whatever its height. */
const bandIsEmpty = (b) => !b.node || elementsOf(b.node).length === 0;

function renderCanvas() {
  index = indexTree(S.root);
  bandList = bands(S.root);
  const page = pageInfo(S.root);
  const sc = scale();
  const sheet = $("sheet");
  const hadFocus = sheet.contains(document.activeElement);
  sheet.textContent = "";

  let y = page.margins.top;
  const rows = bandList.map((b) => {
    const hh = b.node ? num(b.node, "height") : 0;
    const empty = bandIsEmpty(b);
    const shown = empty ? EMPTY_BAND / sc : Math.max(hh, 4 / sc);
    const row = { b, y, h: hh, shown, empty };
    y += shown;
    return row;
  });
  const pageH = Math.max(y + page.margins.bottom, 120);
  const GUTTER = gutter();
  sheet.style.width = `${GUTTER + page.width * sc}px`;

  const pageEl = h("div", { class: "page", style: { left: `${GUTTER}px`, width: `${page.width}px`, height: `${pageH}px`, transform: `scale(${sc})` } });
  pageEl.style.setProperty("--sc", String(sc));
  pageEl.append(h("div", { class: "margin-guide", style: { left: `${page.margins.left}px`, top: `${page.margins.top}px`, width: `${page.columnWidth}px`, height: `${pageH - page.margins.top - page.margins.bottom}px` } }));
  sheet.append(pageEl);

  let labelBottom = 0;
  for (const row of rows) {
    const { b } = row;
    const nm = bandName(b);
    const selected = S.sel && S.sel.type === "band" && S.sel.key === b.key;
    const bandEl = h("div", {
      class: `band${row.empty ? " empty-band" : ""}${S.snap && selected && !row.empty ? " grid" : ""}${selected ? " selected" : ""}${b.kind === "detail" ? " detail" : ""}`,
      style: { left: `${page.margins.left}px`, top: `${row.y}px`, width: `${page.columnWidth}px`, height: `${row.shown}px`, backgroundSize: `${S.grid}px ${S.grid}px` },
      dataset: { bandKey: b.key, ...(b.node ? { cid: String(b.node._id) } : {}) },
    });
    if (!row.empty) {
      renderElements(b.node, bandEl);
      bandEl.append(h("div", { class: "band-resize", title: t("canvas.resizeBand", { band: nm.name }), dataset: { resizeBand: b.key } }));
    }
    pageEl.append(bandEl);

    const title = `${nm.where ? `${nm.name}: ${nm.where}. ` : `${nm.name}. `}${t("band.tech", { name: nm.tech })}`;
    if (row.empty) {
      // An empty band is a slim strip on the page with its name in it: a place to drop on.
      sheet.append(h("button", {
        type: "button", class: `band-strip${selected ? " selected" : ""}${b.kind === "detail" ? " detail" : ""}`, title,
        style: { left: `${GUTTER + page.margins.left * sc}px`, top: `${row.y * sc}px`, width: `${page.columnWidth * sc}px`, height: `${row.shown * sc}px` },
        dataset: { selectBand: b.key, bandKey: b.key },
        "aria-label": `${nm.name}. ${nm.where ? `${nm.where}. ` : ""}${t("canvas.emptyBand")}`,
        "aria-pressed": selected ? "true" : "false",
      }, h("b", {}, nm.name), h("span", { class: "add-here" }, t("canvas.addHere"))));
      continue;
    }
    // Names of bands with something on them go left of the page; thin ones would overlap, so
    // each starts below the one before.
    const top = Math.max(row.y * sc, labelBottom);
    // A long name takes two lines: give it room for them.
    const lines = nm.name.length * 6.9 > GUTTER - 28 ? 2 : 1;
    const room = Math.max(row.shown * sc - (top - row.y * sc), lines === 2 ? 34 : 18);
    labelBottom = top + room;
    sheet.append(h("button", {
      type: "button", class: `band-label${selected ? " selected" : ""}`, title,
      style: { top: `${top}px`, height: `${room}px`, width: `${GUTTER - 14}px` },
      dataset: { selectBand: b.key },
      "aria-label": `${nm.name}. ${nm.where ? `${nm.where}. ` : ""}${t("canvas.bandHeight", { h: row.h })}`,
      "aria-pressed": selected ? "true" : "false",
    }, h("b", {}, nm.name), nm.where && room >= 44 ? h("small", {}, nm.where) : null));
  }

  sheet.style.height = `${Math.max(pageH * sc, labelBottom)}px`;

  if (hadFocus) {
    const target = S.sel && S.sel.type === "el" ? sheet.querySelector(`[data-id="${S.sel.id}"]`)
      : S.sel && S.sel.type === "band" ? sheet.querySelector(`[data-select-band="${CSS.escape(S.sel.key)}"]`) : null;
    if (target) target.focus({ preventScroll: true });
  }
  $("zoom-label").textContent = `${Math.round(S.zoom * 100)}%`;
}

function renderElements(container, into) {
  for (const el of elementsOf(container)) into.append(renderElement(el));
}

const textKinds = new Set(["staticText", "textField"]);

function pill(ref) {
  return h("span", { class: `pill${ref.kind === "F" ? "" : " pill-other"}` }, refLabel(ref));
}

function renderElement(el) {
  const g = geo(el);
  const kind = el.attrs.kind;
  const selected = S.sel && S.sel.type === "el" && S.sel.id === el._id;
  const said = spokenText(el);
  const div = h("div", {
    class: `el${selected ? " selected" : ""}`, tabindex: "0", role: "button",
    "aria-label": `${partName(el)}${said ? `: ${said}` : ""}. ${t("canvas.at", { x: g.x, y: g.y, w: g.w, h: g.h })}`,
    "aria-pressed": selected ? "true" : "false",
    dataset: { id: String(el._id) },
    style: { left: `${g.x}px`, top: `${g.y}px`, width: `${g.w}px`, height: `${g.h}px` },
  });
  const st = effectiveStyle(S.root, el);
  const opaque = st.mode === "Opaque" || (st.mode === undefined && st.backcolor && kind !== "staticText" && kind !== "textField" && kind !== "frame");
  if (textKinds.has(kind)) {
    div.classList.add("text");
    const size = parseFloat(st.fontSize) || 10;
    const font = /mono/i.test(st.fontName || "") ? "var(--code)" : /serif/i.test(st.fontName || "") && !/sans/i.test(st.fontName || "") ? "Georgia, 'DejaVu Serif', serif" : "'DejaVu Sans', Verdana, system-ui, sans-serif";
    Object.assign(div.style, {
      fontSize: `${size}px`, fontFamily: font, fontWeight: st.bold === "true" ? "700" : "400", fontStyle: st.italic === "true" ? "italic" : "normal",
      textDecoration: [st.underline === "true" && "underline", st.strikeThrough === "true" && "line-through"].filter(Boolean).join(" ") || "none",
      color: st.forecolor || "#000", background: st.mode === "Opaque" && st.backcolor ? st.backcolor : "transparent",
      alignItems: { Middle: "center", Bottom: "flex-end" }[st.vTextAlign] || "flex-start",
      textAlign: { Center: "center", Right: "right", Justified: "justify" }[st.hTextAlign] || "left",
    });
    const box = child(el, "box");
    if (box) {
      const pad = (s) => num(box, `${s}Padding`, num(box, "padding", 0));
      div.style.padding = `${pad("top")}px ${pad("right")}px ${pad("bottom")}px ${pad("left")}px`;
      const b = readBorders(el);
      for (const s of ["top", "left", "bottom", "right"]) {
        if (b.sides[s]) div.style[`border${s[0].toUpperCase()}${s.slice(1)}`] = `${Math.max(b.width, .5)}px ${b.style.toLowerCase() === "double" ? "double" : b.style.toLowerCase()} ${b.color}`;
      }
    }
    const span = h("span");
    if (kind === "textField") {
      const expr = ((child(el, "expression") || {}).text || "").trim();
      const shown = expressionSegments(expr);
      if (expr === TODAY) span.append(h("span", { class: "pill pill-other" }, t("pill.today")));
      else if (shown.segs) {
        // Values are pills with the field's name; the rest is the text as it prints.
        for (const s of shown.segs) span.append(s.ref ? pill(s.ref) : s.lit);
        if (!shown.segs.length) span.append(h("span", { class: "pill pill-empty" }, t("pill.empty")));
      } else {
        span.append(h("span", { class: "fx", title: t("canvas.formula") }, "ƒ"));
        for (const s of shown.formula) span.append(s.ref ? pill(s.ref) : h("span", { class: "code" }, s.lit));
      }
    } else {
      span.textContent = (child(el, "text") || {}).text || "";
    }
    div.append(span);
  } else if (kind === "line") {
    div.classList.add("line");
    const pen = readPen(el);
    const color = st.forecolor || "#000";
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    const ln = document.createElementNS("http://www.w3.org/2000/svg", "line");
    const up = el.attrs.direction === "BottomUp";
    // JasperReports draws a 1-point-high line along its top edge, and a 1-point-wide one along its left.
    const flat = g.h <= 1, upright = g.w <= 1 && !flat;
    const [x1, y1, x2, y2] = flat ? [0, 2, g.w, 2] : upright ? [2, 0, 2, g.h] : up ? [0, g.h, g.w, 0] : [0, 0, g.w, g.h];
    Object.entries({ x1, y1, x2, y2, stroke: color, "stroke-width": Math.max(pen.width, .5), "stroke-dasharray": pen.style === "Dashed" ? "4 3" : pen.style === "Dotted" ? "1 2" : "" })
      .forEach(([k, v]) => ln.setAttribute(k, v));
    svg.append(ln);
    div.append(svg);
    if (flat) Object.assign(div.style, { height: "4px", marginTop: "-2px" });
    if (upright) Object.assign(div.style, { width: "4px", marginLeft: "-2px" });
  } else if (kind === "rectangle" || kind === "ellipse") {
    const pen = readPen(el);
    Object.assign(div.style, {
      border: `${Math.max(pen.width, .5)}px ${pen.style.toLowerCase() === "double" ? "double" : pen.style.toLowerCase()} ${st.forecolor || "#000"}`,
      background: (st.mode === "Opaque" || st.mode === undefined) ? (st.backcolor || "#fff") : "transparent",
      borderRadius: kind === "ellipse" ? "50%" : `${num(el, "radius")}px`,
    });
  } else if (kind === "image") {
    div.classList.add("image");
    if (el.attrs.scaleImage === "FillFrame") div.classList.add("fill");
    if (el.attrs.scaleImage === "Clip") div.classList.add("clip");
    const expr = ((child(el, "expression") || {}).text || "").trim();
    const m = /^"(data:image\/[^"]+)"$/.exec(expr);
    if (m) div.append(h("img", { src: m[1], alt: "" }));
    else div.append(expr ? expr.replace(/^"|"$/g, "") : t("canvas.imageEmpty"));
  } else {
    const inner = innerContainer(el);
    if (inner) {
      div.classList.add("container");
      if (st.mode === "Opaque" && st.backcolor) div.style.background = st.backcolor;
      if (inner === el) {
        div.dataset.cid = String(el._id);
        renderElements(el, div);
      } else {
        const contents = h("div", { class: "contents", dataset: { cid: String(inner._id) }, style: { position: "absolute", left: "0", top: "0", width: `${num(inner, "width", g.w)}px`, height: `${num(inner, "height", g.h)}px` } });
        renderElements(inner, contents);
        div.append(contents);
      }
      div.append(h("span", { class: "el-tag" }, partName(el)));
    } else {
      div.classList.add("other");
      div.append(h("span", { class: "el-tag" }, partName(el)));
    }
    if (opaque && !inner) div.style.background = st.backcolor;
  }
  if (selected) {
    const hs = 7 / scale(); // handles stay the same size on screen
    for (const [k, l, tp] of [["nw", 0, 0], ["n", .5, 0], ["ne", 1, 0], ["e", 1, .5], ["se", 1, 1], ["s", .5, 1], ["sw", 0, 1], ["w", 0, .5]]) {
      if (kind === "line" && g.h <= 1 && !["e", "w"].includes(k)) continue;
      if (kind === "line" && g.w <= 1 && g.h > 1 && !["n", "s"].includes(k)) continue;
      div.append(h("div", { class: "handle", dataset: { h: k }, style: { width: `${hs}px`, height: `${hs}px`, left: `calc(${l * 100}% - ${hs / 2}px)`, top: `calc(${tp * 100}% - ${hs / 2}px)` } }));
    }
  }
  return div;
}

// ---------------------------------------------------------------------------------------------
// Selecting, moving and resizing with the mouse

function select(sel, { focus = false } = {}) {
  S.sel = sel;
  S.whenCustom = null;
  S.coalesce = null;
  renderCanvas();
  renderProps();
  if (focus) {
    const tg = sel && sel.type === "el" ? $("sheet").querySelector(`[data-id="${sel.id}"]`) : sel && sel.type === "band" ? $("sheet").querySelector(`[data-select-band="${CSS.escape(sel.key)}"]`) : null;
    if (tg) tg.focus({ preventScroll: true });
  }
}

/** The band, frame or list contents under a point of the screen (not `skip` or what is inside it). */
function containerAt(clientX, clientY, skip) {
  for (const node of document.elementsFromPoint(clientX, clientY)) {
    if (skip && (node === skip || skip.contains(node))) continue;
    const c = node.closest("[data-cid], [data-band-key]");
    if (!c || (skip && (c === skip || skip.contains(c)))) continue;
    if (!$("sheet").contains(c)) continue;
    return c;
  }
  return null;
}

/** The node behind a container's DOM element, adding the band to the template if it is new. */
function containerNode(div, create = true) {
  if (div.dataset.cid) return index.get(Number(div.dataset.cid)).node;
  const b = bandList.find((x) => x.key === div.dataset.bandKey);
  if (!b) return null;
  if (b.node) return b.node;
  return create && b.create ? b.create() : null;
}

function startPointer(e) {
  if (e.button !== 0) return;
  const handle = e.target.closest(".handle");
  const elDiv = e.target.closest(".el");
  const bandResize = e.target.closest("[data-resize-band]");
  const bandLabel = e.target.closest("[data-select-band]");
  if (bandLabel) {
    select({ type: "band", key: bandLabel.dataset.selectBand }, { focus: true });
    return;
  }
  if (bandResize) return startBandResize(e, bandResize.dataset.resizeBand);
  if (elDiv) {
    e.preventDefault();
    const id = Number(elDiv.dataset.id);
    if (!(S.sel && S.sel.type === "el" && S.sel.id === id)) select({ type: "el", id });
    const div = $("sheet").querySelector(`[data-id="${id}"]`);
    div.focus({ preventScroll: true });
    if (handle) return startResize(e, id, handle.dataset.h);
    return startMove(e, id);
  }
  const bandDiv = e.target.closest("[data-band-key]");
  if (bandDiv) {
    select({ type: "band", key: bandDiv.dataset.bandKey });
    $("canvas-stage").focus({ preventScroll: true });
    return;
  }
  if (e.target.closest("#canvas-stage")) select(null);
}

function track(onMove, onUp) {
  const move = (e) => onMove(e);
  const up = (e) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", up);
    document.body.style.cursor = "";
    onUp(e);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", up);
}

function startMove(e, id) {
  const { node: el } = index.get(id);
  const start = geo(el);
  const sx = e.clientX, sy = e.clientY;
  let moved = false;
  const div = $("sheet").querySelector(`[data-id="${id}"]`);
  let target = null;
  track((ev) => {
    const dx = (ev.clientX - sx) / scale(), dy = (ev.clientY - sy) / scale();
    if (!moved && Math.abs(dx) + Math.abs(dy) < 2) return;
    moved = true;
    div.style.left = `${snapV(start.x + dx)}px`;
    div.style.top = `${snapV(start.y + dy)}px`;
    const over = containerAt(ev.clientX, ev.clientY, div);
    if (over !== target) {
      if (target) target.classList.remove("drop");
      target = over;
      const parentDiv = div.parentElement.closest("[data-cid], [data-band-key]");
      if (target && target !== parentDiv) target.classList.add("drop");
    }
  }, () => {
    if (target) target.classList.remove("drop");
    if (!moved) return;
    change(() => {
      const parent = index.get(id).parent;
      let into = parent;
      if (target) into = containerNode(target) || parent;
      if (into !== parent && into.tag === "contents" && el.attrs.kind === "component") into = parent; // no lists in lists yet
      if (into !== parent) {
        // Keep the part where it was dropped, measured from its new container.
        const r = div.getBoundingClientRect();
        const tr = (target.querySelector(":scope > .contents") || target).getBoundingClientRect();
        parent.children.splice(parent.children.indexOf(el), 1);
        into.children.push(el);
        index = indexTree(S.root);
        setGeo(el, { x: snapV((r.left - tr.left) / scale()), y: snapV((r.top - tr.top) / scale()) });
        if (el.attrs.kind === "line" && geo(el).h <= 1) setGeo(el, { y: snapV((r.top + 2 - tr.top) / scale()) });
      } else {
        setGeo(el, { x: parseFloat(div.style.left), y: parseFloat(div.style.top) });
      }
      fit(el, into);
    });
  });
}

function startResize(e, id, hd) {
  const { node: el, parent } = index.get(id);
  const s = geo(el);
  const sx = e.clientX, sy = e.clientY;
  const div = $("sheet").querySelector(`[data-id="${id}"]`);
  const min = el.attrs.kind === "line" ? 0 : 4;
  let g = { ...s };
  track((ev) => {
    const dx = (ev.clientX - sx) / scale(), dy = (ev.clientY - sy) / scale();
    g = { ...s };
    if (hd.includes("e")) g.w = Math.max(min, snapV(s.x + s.w + dx) - s.x);
    if (hd.includes("s")) g.h = Math.max(min, snapV(s.y + s.h + dy) - s.y);
    if (hd.includes("w")) { const nx = Math.min(snapV(s.x + dx), s.x + s.w - min); g.w = s.x + s.w - nx; g.x = nx; }
    if (hd.includes("n")) { const ny = Math.min(snapV(s.y + dy), s.y + s.h - min); g.h = s.y + s.h - ny; g.y = ny; }
    Object.assign(div.style, { left: `${g.x}px`, top: `${g.y}px`, width: `${g.w}px`, height: `${g.h}px` });
  }, () => {
    if (g.x === s.x && g.y === s.y && g.w === s.w && g.h === s.h) return;
    change(() => {
      setGeo(el, g);
      fit(el, parent);
    });
  });
}

function startBandResize(e, key) {
  e.preventDefault();
  const b = bandList.find((x) => x.key === key);
  const startH = b.node ? num(b.node, "height") : 0;
  const minH = b.node ? elementsOf(b.node).reduce((m, el) => Math.max(m, num(el, "y") + num(el, "height")), 0) : 0;
  const sy = e.clientY;
  const bandDiv = $("sheet").querySelector(`[data-band-key="${CSS.escape(key)}"]`);
  const handle = bandDiv.querySelector(".band-resize");
  handle.classList.add("active");
  document.body.style.cursor = "ns-resize";
  let hh = startH;
  track((ev) => {
    hh = Math.max(minH, snapV(startH + (ev.clientY - sy) / scale()));
    bandDiv.style.height = `${Math.max(hh, 4 / scale())}px`;
  }, () => {
    handle.classList.remove("active");
    if (hh === startH) return;
    S.sel = { type: "band", key };
    change(() => {
      const node = b.node || b.create();
      node.attrs.height = String(hh);
    });
    announce(t("msg.bandHeight", { band: bandName(b).name, h: hh }));
  });
}

// ---------------------------------------------------------------------------------------------
// Keyboard on the page

function onKey(e) {
  const mod = e.ctrlKey || e.metaKey;
  const inField = e.target.closest("input, textarea, select, [contenteditable]");
  if (mod && !inField && e.key.toLowerCase() === "z") {
    e.preventDefault();
    return e.shiftKey ? redo() : undo();
  }
  if (mod && !inField && e.key.toLowerCase() === "y") {
    e.preventDefault();
    return redo();
  }
  if (inField || !e.target.closest("#canvas-stage")) return;
  const sel = S.sel && S.sel.type === "el" ? index.get(S.sel.id) : null;
  if (e.key === "Escape") {
    e.preventDefault();
    select(null);
    $("canvas-stage").focus();
    return;
  }
  if (!sel) return;
  const el = sel.node;
  if (e.key === "Delete" || e.key === "Backspace") {
    e.preventDefault();
    return removeSelected();
  }
  if (mod && e.key.toLowerCase() === "d") {
    e.preventDefault();
    return duplicateSelected();
  }
  if (e.key === "Enter" && textKinds.has(el.attrs.kind)) {
    e.preventDefault();
    const box = $("prop-text") || $("prop-expr");
    if (box) box.focus();
    return;
  }
  const dirs = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
  if (dirs[e.key]) {
    e.preventDefault();
    const [dx, dy] = dirs[e.key];
    const step = e.altKey || !S.snap ? 1 : S.grid;
    change(() => {
      const g = geo(el);
      if (e.shiftKey) {
        g.w = Math.max(1, g.w + dx * step);
        g.h = Math.max(el.attrs.kind === "line" ? 0 : 1, g.h + dy * step);
      } else {
        g.x += dx * step;
        g.y += dy * step;
      }
      setGeo(el, g);
      fit(el, sel.parent);
    }, { coalesce: `key:${el._id}:${e.shiftKey ? "size" : "move"}` });
    const g = geo(el);
    announce(e.shiftKey ? t("msg.size", { w: g.w, h: g.h }) : t("msg.at", { x: g.x, y: g.y }));
  }
}

function removeSelected() {
  if (!S.sel || S.sel.type !== "el") return;
  const { node, parent } = index.get(S.sel.id);
  const label = partName(node);
  S.sel = null;
  change(() => { parent.children.splice(parent.children.indexOf(node), 1); });
  announce(t("msg.removed", { part: label }));
  $("canvas-stage").focus();
}

function duplicateSelected() {
  if (!S.sel || S.sel.type !== "el") return;
  const { node, parent } = index.get(S.sel.id);
  const copy = reId(JSON.parse(JSON.stringify(node)));
  const g = geo(node);
  const spot = { x: g.x + 10, y: g.y + 10 };
  change(() => {
    parent.children.splice(parent.children.indexOf(node) + 1, 0, copy);
    index = indexTree(S.root);
    setGeo(copy, spot);
    fit(copy, parent);
    S.sel = { type: "el", id: copy._id };
  });
  select(S.sel, { focus: true });
  announce(t("msg.duplicated"));
}

function reorder(toFront) {
  const { node, parent } = index.get(S.sel.id);
  change(() => {
    const els = parent.children;
    els.splice(els.indexOf(node), 1);
    if (toFront) els.push(node);
    else els.splice(els.findIndex((c) => c.tag === "element") === -1 ? els.length : els.findIndex((c) => c.tag === "element"), 0, node);
  });
}

// ---------------------------------------------------------------------------------------------
// Adding fields and parts

const humanize = (name) => name.replace(/[_-]+/g, " ").replace(/([a-z])([A-Z])/g, "$1 $2").replace(/^./, (c) => c.toUpperCase());

/** Where a part goes when it is added without the mouse: the selected band, or the rows. */
function defaultTarget() {
  if (S.sel && S.sel.type === "el") {
    const { node, parent } = index.get(S.sel.id);
    return innerContainer(node) || parent;
  }
  const key = S.sel && S.sel.type === "band" ? S.sel.key : "detail:0";
  const b = bandList.find((x) => x.key === key) || bandList.find((x) => x.kind === "detail");
  return b.node || b.create();
}

/** Adds a field or a part to `container` (at x, y, or in the first free spot). Returns the part. */
function addThing(payload, container, at) {
  let added = null;
  change(() => {
    index = indexTree(S.root);
    bandList = bands(S.root);
    added = payload.type === "field" ? addField(payload.field, container, at) : addPart(payload.part, container, at);
    if (!added) return false;
    index = indexTree(S.root);
    S.sel = { type: "el", id: added._id };
  });
  if (added) {
    select(S.sel, { focus: true });
    if (payload.type === "part" && payload.part === "staticText") {
      const box = $("prop-text");
      if (box) { box.focus(); box.select(); }
    }
    if (payload.type === "part" && payload.part === "field") {
      const box = $("prop-text");
      if (box) { box.focus(); openPicker(box, null); }
    }
    if (payload.type === "part" && payload.part === "image") $("prop-image-file")?.click();
  }
  return added;
}

function place(container, w, hh, at) {
  return at ? { x: snapV(at.x), y: snapV(at.y) } : freeSpot(container, w, hh);
}

function addField(f, container, at) {
  const ds = datasetOfContainer(S.root, container, index);
  if (f.param) {
    if (ds) { announce(t("msg.paramInList"), "bad"); return null; }
    const p = place(container, 140, 20, at);
    const el = newElement("textField", p.x, p.y, 140, 20, { blankWhenNull: "true" });
    setMainText(el, `$P{${f.name}}`);
    container.children.push(el);
    index = indexTree(S.root);
    fit(el, container);
    return el;
  }
  if (f.type === "List") {
    if (ds) { announce(t("msg.listInList"), "bad"); return null; }
    return addList(f, container, at);
  }
  if (f.list && !ds) {
    const hasList = [...index.values()].some((x) => x.node.tag === "contents" && listFieldOf(x.node) === f.list);
    announce(t(hasList ? "msg.itemFieldHasList" : "msg.itemFieldNoList", { name: f.name, list: f.list }), "bad");
    return null;
  }
  if (!f.list && ds) {
    announce(t("msg.outerFieldInList", { name: f.name }), "bad");
    return null;
  }
  ensureField(ds || S.root, f.name, f.class, !ds);
  const numeric = f.type === "Number" || f.type === "Whole number";
  const w = numeric || f.type === "Date" ? 90 : 140;
  const hh = 20;
  const p = place(container, w, hh, at);
  const attrs = {};
  if (f.type === "Number") attrs.pattern = "#,##0.00";
  if (f.type === "Date") attrs.pattern = "MMM d, yyyy";
  if (numeric) attrs.hTextAlign = "Right";
  if (f.type === "Text") attrs.textAdjust = "StretchHeight";
  attrs.blankWhenNull = "true";
  const el = newElement("textField", p.x, p.y, w, hh, attrs);
  setMainText(el, `$F{${f.name}}`);
  container.children.push(el);
  index = indexTree(S.root);
  fit(el, container);
  // A field dropped on the rows gets its column title above it.
  const detailBand = bandList.find((b) => b.kind === "detail" && b.node === container);
  if (store.get(HEADINGS) !== "no" && detailBand) {
    const chEntry = bandList.find((b) => b.kind === "columnHeader");
    const ch = chEntry && (chEntry.node || chEntry.create());
    if (ch) {
      if (num(ch, "height") < 20) ch.attrs.height = "20";
      const g = geo(el);
      const taken = elementsOf(ch).some((o) => { const og = geo(o); return g.x < og.x + og.w && g.x + g.w > og.x; });
      if (!taken) {
        const head = newElement("staticText", g.x, 0, g.w, Math.min(20, num(ch, "height")), { bold: "true", ...(numeric ? { hTextAlign: "Right" } : {}) });
        setMainText(head, humanize(f.name));
        ch.children.push(head);
      }
    }
  }
  return el;
}

function addList(f, container, at) {
  const cw = containerWidth(container);
  const p = place(container, cw, 20, at);
  const width = Math.max(40, cw - p.x);
  const names = new Set(kids(S.root, "dataset").map((d) => d.attrs.name));
  let name = f.name;
  for (let i = 2; names.has(name); i++) name = `${f.name}${i}`;
  const sub = (f.fields || []).slice(0, 6);
  const dataset = make("dataset", { name }, sub.map((s) => make("field", { name: s.name, class: s.class })));
  insertOrdered(S.root, dataset);
  ensureField(S.root, f.name, "java.util.List", true);
  const colW = sub.length ? Math.floor(width / sub.length) : width;
  const cells = sub.map((s, i) => {
    const numeric = s.type === "Number" || s.type === "Whole number";
    const el = newElement("textField", i * colW, 0, i === sub.length - 1 ? width - i * colW : colW, 20, {
      ...(s.type === "Number" ? { pattern: "#,##0.00" } : {}), ...(s.type === "Date" ? { pattern: "MMM d, yyyy" } : {}),
      ...(numeric ? { hTextAlign: "Right" } : {}), blankWhenNull: "true",
    });
    setMainText(el, `$F{${s.name}}`);
    return el;
  });
  const list = make("element", { kind: "component", x: String(p.x), y: String(p.y), width: String(width), height: "20" }, [
    make("component", { kind: "list", printOrder: "Vertical" }, [
      make("datasetRun", { subDataset: name }, [
        make("dataSourceExpression", {}, [], `new net.sf.jasperreports.engine.data.JRMapCollectionDataSource($F{${f.name}})`),
      ]),
      make("contents", { width: String(width), height: "20" }, cells),
    ]),
  ]);
  container.children.push(list);
  index = indexTree(S.root);
  fit(list, container);
  announce(t("msg.listAdded", { name: f.name }));
  return list;
}

const PART_SIZES = { staticText: [120, 20], field: [140, 20], line: [200, 1], rectangle: [120, 60], ellipse: [60, 60], image: [80, 60], pageNumber: [100, 20], today: [110, 20] };

function addPart(part, container, at) {
  const [w, hh] = PART_SIZES[part];
  const p = place(container, w, hh, at);
  let el;
  if (part === "staticText") {
    el = newElement("staticText", p.x, p.y, w, hh);
    setMainText(el, t("part.textDefault"));
  } else if (part === "field") {
    el = newElement("textField", p.x, p.y, w, hh, { blankWhenNull: "true" });
    setMainText(el, '""');
  } else if (part === "pageNumber") {
    el = newElement("textField", p.x, p.y, w, hh, { hTextAlign: "Right" });
    setMainText(el, `${javaString(t("part.pageWord"))} + $V{PAGE_NUMBER}`);
  } else if (part === "today") {
    el = newElement("textField", p.x, p.y, w, hh, { pattern: "MMM d, yyyy" });
    setMainText(el, TODAY);
  } else if (part === "image") {
    el = newElement("image", p.x, p.y, w, hh, { scaleImage: "RetainShape" });
  } else {
    el = newElement(part, p.x, p.y, w, hh);
  }
  container.children.push(el);
  index = indexTree(S.root);
  fit(el, container);
  return el;
}

// Drag and drop from the side panel onto the page.
function startDrag(e, payload) {
  dragPayload = payload;
  e.dataTransfer.effectAllowed = "copy";
  e.dataTransfer.setData("text/plain", payload.type === "field" ? payload.field.name : payload.part);
}
let ghost = null, dropTarget = null;
function clearDrop() {
  if (ghost) ghost.remove();
  ghost = null;
  if (dropTarget) dropTarget.classList.remove("drop");
  dropTarget = null;
}
function dragSize(payload) {
  if (payload.type === "part") return PART_SIZES[payload.part];
  const tp = payload.field.type;
  return tp === "List" ? [200, 20] : tp === "Number" || tp === "Whole number" || tp === "Date" ? [90, 20] : [140, 20];
}
function onDragOver(e) {
  if (!dragPayload) return;
  const target = containerAt(e.clientX, e.clientY);
  if (!target) return clearDrop();
  e.preventDefault();
  e.dataTransfer.dropEffect = "copy";
  if (target !== dropTarget) {
    clearDrop();
    dropTarget = target;
    target.classList.add("drop");
  }
  const empty = target.classList.contains("band-strip");
  // The strip of an empty band is outside the page: the ghost goes in the band itself.
  const holder = empty ? $("sheet").querySelector(`.band[data-band-key="${CSS.escape(target.dataset.bandKey)}"]`) : target.querySelector(":scope > .contents") || target;
  const r = holder.getBoundingClientRect();
  const [w, hh] = dragSize(dragPayload);
  const x = snapV((e.clientX - r.left) / scale() - Math.min(w, 40) / 2), y = empty ? 0 : snapV((e.clientY - r.top) / scale() - hh / 2);
  if (!ghost) ghost = h("div", { class: "drop-ghost" });
  holder.append(ghost);
  Object.assign(ghost.style, { left: `${Math.max(0, x)}px`, top: `${Math.max(0, y)}px`, width: `${w}px`, height: `${Math.max(hh, 2)}px` });
}
function onDrop(e) {
  if (!dragPayload) return;
  e.preventDefault();
  const target = dropTarget || containerAt(e.clientX, e.clientY);
  const at = ghost ? { x: parseFloat(ghost.style.left), y: parseFloat(ghost.style.top) } : null;
  clearDrop();
  const payload = dragPayload;
  dragPayload = null;
  if (!target) return;
  const node = containerNode(target);
  if (node) addThing(payload, node, at || { x: 0, y: 0 });
}

// ---------------------------------------------------------------------------------------------
// Data and the field list

function setData(text, record = true) {
  S.dataText = text;
  if ($("data").value !== text) $("data").value = text;
  try {
    const parsed = text.trim() ? JSON.parse(text) : [];
    S.rows = sampleRows(parsed);
    S.dataFields = detectFields(S.rows);
    dataError = null;
  } catch (err) {
    dataError = err.message.replace(/^JSON\.parse: /, "");
  }
  renderDataMsg();
  if (dataError) return;
  if (record) {
    renderFields();
    schedulePreview();
    scheduleSave();
  }
}

function renderDataMsg() {
  const msg = $("data-msg");
  if (dataError) {
    msg.textContent = t("data.invalid", { error: dataError });
    msg.className = "msg bad";
  } else {
    msg.textContent = t("data.count", { rows: S.rows.length, fields: S.dataFields.length });
    msg.className = "msg";
  }
}

function setParams(text, record = true) {
  S.paramsText = text;
  if ($("params").value !== text) $("params").value = text;
  const msg = $("params-msg");
  try {
    const v = text.trim() ? JSON.parse(text) : {};
    if (!v || typeof v !== "object" || Array.isArray(v)) throw new Error(t("data.paramsObject"));
    S.params = v;
    msg.textContent = "";
    msg.className = "msg";
  } catch (err) {
    msg.textContent = t("data.invalid", { error: err.message });
    msg.className = "msg bad";
    return;
  }
  if (record) {
    schedulePreview();
    scheduleSave();
  }
}

function fieldButton(f, payload, onclick) {
  return h("button", {
    type: "button", class: "field-btn", draggable: "true",
    title: f.type === "List" ? t("fields.dragList", { name: f.name }) : t("fields.drag"),
    "aria-label": `${f.name}, ${typeName(f.type)}${f.list ? `, ${t("fields.inEach", { list: f.list })}` : ""}. ${t("fields.add")}`,
    ondragstart: (e) => startDrag(e, payload),
    ondragend: () => { dragPayload = null; clearDrop(); },
    onclick,
  }, h("span", { class: "grip", "aria-hidden": "true" }), h("span", { class: "name" }, f.name), h("span", { class: "type", dataset: { t: f.type } }, typeName(f.type)));
}

function renderParams() {
  const params = kids(S.root, "parameter").map((p) => ({ name: p.attrs.name, class: p.attrs.class || "java.lang.String", type: CLASS_TYPES[p.attrs.class] || "Text", param: true }));
  $("params-section").hidden = params.length === 0;
  const ul = $("param-list");
  ul.textContent = "";
  for (const f of params) {
    const payload = { type: "field", field: f };
    ul.append(h("li", {}, fieldButton(f, payload, () => addThing(payload, defaultTarget(), null))));
  }
  // Files the template imports are already in it; images and subreports are what the preview needs.
  const names = Object.keys(S.resources).filter((n) => !/\.(ferro|jrtx)$/i.test(n));
  const note = $("files-note");
  note.hidden = names.length === 0;
  note.textContent = names.length ? t("data.files", { files: names.join(", ") }) : "";
}

function renderFields() {
  renderParams();
  const ul = $("fields");
  ul.textContent = "";
  const declared = declaredFields(S.root);
  const known = new Set(S.dataFields.map((f) => f.name));
  const fromTemplate = declared.filter((f) => !known.has(f.name));
  const all = [...S.dataFields, ...fromTemplate.map((f) => ({ ...f, fromTemplate: true }))];
  if (!all.length) {
    ul.append(h("li", { class: "empty" }, t("fields.none")));
    return;
  }
  const item = (f) => {
    const payload = { type: "field", field: f };
    const btn = fieldButton(f, payload, () => {
      let target = defaultTarget();
      if (f.list) {
        // Put it in the list of its items, if there is one.
        const list = [...index.values()].map((x) => x.node).find((n) => n.tag === "contents" && listFieldOf(n) === f.list);
        if (list) target = list;
      }
      addThing(payload, target, null);
    });
    const li = h("li", {}, btn);
    if (f.fromTemplate) li.append(h("p", { class: "note" }, t("fields.notInData")));
    if (f.fields && f.fields.length) li.append(h("ul", { "aria-label": t("fields.inEach", { list: f.name }) }, f.fields.map((s) => item({ ...s, list: f.name }))));
    return li;
  };
  all.forEach((f) => ul.append(item(f)));
}

/** For a list's contents, the field of the report whose items it repeats ($F{lines} → "lines"). */
function listFieldOf(contents) {
  const comp = index.get(contents._id).parent;
  const run = child(comp, "datasetRun");
  const expr = run && child(run, "dataSourceExpression");
  const m = expr && /\$F\{([^}]+)\}/.exec(expr.text || "");
  return m ? m[1] : null;
}

/** The fields a part can show: those of its list's items inside a list, else the report's and the data's. */
function fieldsFor(el) {
  const { parent } = index.get(el._id);
  const ds = datasetOfContainer(S.root, parent, index);
  if (ds) {
    const items = S.dataFields.find((f) => f.name === listFieldOf(findContents(parent)))?.fields || [];
    const declared = declaredFields(ds);
    return { holder: ds, isRoot: false, fields: [...declared, ...items.filter((f) => !declared.some((d) => d.name === f.name))] };
  }
  const declared = declaredFields(S.root);
  return { holder: S.root, isRoot: true, fields: [...S.dataFields.filter((f) => f.type !== "List"), ...declared.filter((d) => d.type !== "List" && !S.dataFields.some((f) => f.name === d.name))] };
}
function findContents(n) {
  let c = n;
  while (c && c.tag !== "contents") c = index.get(c._id)?.parent;
  return c;
}

/** Declares the fields a text names, so the template compiles. */
function declareFieldsOf(el, expr) {
  const { holder, isRoot, fields } = fieldsFor(el);
  for (const s of parseText(expressionToText(expr) ?? "")) {
    if (!s.ref || s.ref.kind !== "F") continue;
    const f = fields.find((x) => x.name === s.ref.name);
    ensureField(holder, s.ref.name, f ? f.class : "java.lang.Object", isRoot);
  }
}

// ---------------------------------------------------------------------------------------------
// The Text box: words, and fields as {name}. Typing { (or the Insert button) opens a list of
// the fields to choose from.

let picker = null; // { box, start, items, active, el }

function pickerItems(el, query) {
  const { fields } = fieldsFor(el);
  const items = fields.map((f) => ({ label: f.name, note: typeName(f.type), insert: { kind: "F", name: f.name } }));
  const { parent } = index.get(el._id);
  if (!datasetOfContainer(S.root, parent, index)) {
    for (const p of kids(S.root, "parameter")) items.push({ label: p.attrs.name, note: t("picker.reportValue"), insert: { kind: "P", name: p.attrs.name } });
  }
  items.push({ label: t("pill.page"), note: t("picker.builtIn"), insert: { kind: "V", name: "PAGE_NUMBER" } });
  items.push({ label: t("pill.row"), note: t("picker.builtIn"), insert: { kind: "V", name: "REPORT_COUNT" } });
  const q = (query || "").toLowerCase();
  return q ? items.filter((i) => i.label.toLowerCase().includes(q)) : items;
}

/** Opens the list under the Text box. `start` is where a typed { is, or null to insert at the caret. */
function openPicker(box, start) {
  const el = index.get(S.sel.id).node;
  closePicker();
  const query = start === null ? "" : box.value.slice(start + 1, box.selectionStart);
  const items = pickerItems(el, query);
  if (!items.length && start !== null) return;
  const list = h("ul", { class: "picker", role: "listbox", id: "picker", "aria-label": t("picker.label") });
  picker = { box, start, items, active: 0, list, el };
  box.setAttribute("aria-expanded", "true");
  box.setAttribute("aria-controls", "picker");
  drawPicker();
  box.parentElement.append(list);
}
function drawPicker() {
  const { list, items, active } = picker;
  list.textContent = "";
  if (!items.length) list.append(h("li", { class: "picker-empty" }, t("picker.none")));
  items.forEach((it, i) => list.append(h("li", {
    role: "option", id: `pick-${i}`, "aria-selected": String(i === active), class: i === active ? "active" : "",
    onmousedown: (e) => { e.preventDefault(); choose(i); },
  }, h("span", { class: `pill${it.insert.kind === "F" ? "" : " pill-other"}` }, it.label), h("small", {}, it.note))));
  picker.box.setAttribute("aria-activedescendant", items.length ? `pick-${active}` : "");
  list.querySelector(".active")?.scrollIntoView({ block: "nearest" });
}
function closePicker() {
  if (!picker) return;
  picker.list.remove();
  picker.box.setAttribute("aria-expanded", "false");
  picker.box.removeAttribute("aria-activedescendant");
  picker = null;
}
function choose(i) {
  const { box, start, items } = picker;
  const it = items[i];
  if (!it) return;
  const { kind, name } = it.insert;
  const short = kind === "F" && /^[\p{Alphabetic}_][\p{Alphabetic}\p{N}_]*(\.[\p{Alphabetic}_][\p{Alphabetic}\p{N}_]*)*$/u.test(name);
  const ref = short ? `{${name}}` : `$${kind}{${name}}`;
  const from = start === null ? box.selectionStart : start;
  const to = box.selectionEnd;
  box.value = box.value.slice(0, from) + ref + box.value.slice(to);
  const caret = from + ref.length;
  closePicker();
  box.focus();
  box.setSelectionRange(caret, caret);
  box.dispatchEvent(new Event("input", { bubbles: true }));
}
function pickerKey(e) {
  if (!picker) return;
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    picker.active = (picker.active + (e.key === "ArrowDown" ? 1 : -1) + picker.items.length) % Math.max(1, picker.items.length);
    drawPicker();
  } else if (e.key === "Enter" || e.key === "Tab") {
    if (!picker.items.length) return closePicker();
    e.preventDefault();
    choose(picker.active);
  } else if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    closePicker();
  }
}
function pickerFollow(box) {
  // Typing { opens the list; the letters after it narrow it.
  const caret = box.selectionStart;
  const v = box.value;
  if (picker && picker.start !== null) {
    if (caret <= picker.start || /[\s}]/.test(v.slice(picker.start + 1, caret))) return closePicker();
    picker.items = pickerItems(picker.el, v.slice(picker.start + 1, caret));
    picker.active = 0;
    if (!picker.items.length) return closePicker();
    return drawPicker();
  }
  if (v[caret - 1] === "{" && v[caret - 2] !== "\\" && v[caret - 2] !== "{") openPicker(box, caret - 1);
}

/** Writes what the Text box says into a text part, making a static text a field when it names one. */
function applyText(el, text) {
  const r = textToExpression(text);
  if (el.attrs.kind === "staticText" && r.static !== undefined) {
    setMainText(el, r.static);
    return false;
  }
  const expr = r.expression !== undefined ? r.expression : javaString(r.static);
  let became = false;
  if (el.attrs.kind === "staticText") {
    // A text with a value in it is a text field.
    el.children = el.children.filter((c) => c.tag !== "text");
    el.attrs.kind = "textField";
    el.attrs.blankWhenNull = "true";
    became = true;
  }
  setMainText(el, expr);
  declareFieldsOf(el, expr);
  return became;
}

// ---------------------------------------------------------------------------------------------
// Properties panel

let uid = 0;
const nextId = () => `p${++uid}`;

function field(labelText, control, help) {
  control.id = control.id || nextId();
  return h("div", { class: "prop" }, h("label", { for: control.id }, labelText), control, help ? h("span", { class: "help" }, help) : null);
}
function seg(labelText, options, value, onPick) {
  const group = h("div", { class: "seg", role: "group", "aria-label": labelText });
  for (const [v, text, aria] of options) {
    const label = aria || (typeof text === "string" ? text : "");
    group.append(h("button", { type: "button", "aria-pressed": String(v === value), "aria-label": label, title: label, onclick: () => onPick(v) }, text));
  }
  return h("div", { class: "prop" }, h("span", { class: "label" }, labelText), group);
}
function numberInput(value, onValue, attrs = {}) {
  return h("input", { type: "number", value: String(value), step: "1", ...attrs, oninput: (e) => { const v = parseFloat(e.target.value); if (Number.isFinite(v)) onValue(v); } });
}
function colorInput(value, onValue, label) {
  return h("input", { type: "color", value: normalizeColor(value), "aria-label": label, oninput: (e) => onValue(e.target.value.toUpperCase()) });
}
function normalizeColor(c) {
  if (!c) return "#000000";
  if (/^#[0-9a-f]{6}$/i.test(c)) return c.toLowerCase();
  if (/^#[0-9a-f]{3}$/i.test(c)) return `#${c.slice(1).split("").map((x) => x + x).join("")}`.toLowerCase();
  const named = { black: "#000000", white: "#ffffff", red: "#ff0000", blue: "#0000ff", gray: "#808080", grey: "#808080" };
  return named[c.toLowerCase()] || "#000000";
}
function check(labelText, checked, onChange, attrs = {}) {
  return h("label", { class: "inline" }, h("input", { type: "checkbox", checked, onchange: (e) => onChange(e.target.checked), ...attrs }), labelText);
}

/** A section of the panel, with a heading; `fold` makes it one that opens and closes. */
function section(key, title, { fold = false, open = false } = {}, ...content) {
  if (!fold) return h("section", { class: "psec", dataset: { sec: key } }, h("h3", {}, title), ...content);
  const d = h("details", { class: "psec fold", dataset: { sec: key }, open: open || S.open.has(key) }, h("summary", {}, h("h3", {}, title)), ...content);
  d.addEventListener("toggle", () => { if (d.open) S.open.add(key); else S.open.delete(key); });
  return d;
}

/** Identifies a control of the panel across a redraw, to keep the keyboard focus on it. */
function focusKey(el) {
  const prop = el.closest(".prop, .grid2, .grid4, .colors");
  const label = prop ? (prop.querySelector(".label, label") || {}).textContent : "";
  return [el.tagName, el.id, el.getAttribute("aria-label"), el.tagName === "BUTTON" ? el.textContent : "", label].join("|");
}

function renderProps() {
  closePicker();
  const form = $("props");
  const active = form.contains(document.activeElement) ? document.activeElement : null;
  const key = active ? focusKey(active) : null;
  const caret = active && "selectionStart" in active && typeof active.selectionStart === "number" ? [active.selectionStart, active.selectionEnd] : null;
  const scroll = $("props-side").scrollTop;
  form.textContent = "";
  if (S.sel && S.sel.type === "el" && index.has(S.sel.id)) elementProps(form, index.get(S.sel.id).node);
  else if (S.sel && S.sel.type === "band" && bandList.some((x) => x.key === S.sel.key)) bandProps(form, bandList.find((x) => x.key === S.sel.key));
  else reportProps(form);
  $("props-side").scrollTop = scroll;
  if (key) {
    const again = [...form.querySelectorAll("button, input, select, textarea, summary")].find((x) => focusKey(x) === key);
    if (again) {
      again.focus({ preventScroll: true });
      if (caret && "setSelectionRange" in again) try { again.setSelectionRange(...caret); } catch { /* not a text box */ }
    }
  }
}

function setTitle(text, sub) {
  $("props-h").textContent = text;
  const p = $("props-sub");
  p.textContent = sub || "";
  p.hidden = !sub;
}

function reportProps(form) {
  const root = S.root;
  const page = pageInfo(root);
  setTitle(t("props.page"), t("props.pageHint"));
  const sizeSel = h("select", { onchange: (e) => change(() => {
    const [w, hh] = PAGE_SIZES[e.target.value];
    setPage(root, { width: page.landscape ? hh : w, height: page.landscape ? w : hh, margins: page.margins });
  }) }, [...Object.keys(PAGE_SIZES), ...(page.size === "Custom" ? ["Custom"] : [])].map((k) => h("option", { value: k, selected: k === page.size }, k === "Custom" ? t("props.customSize", { w: page.width, h: page.height }) : k)));
  const margins = h("div", { class: "grid4" });
  for (const k of ["top", "bottom", "left", "right"]) {
    margins.append(h("label", {}, t(`props.${k}`), numberInput(page.margins[k], (v) => change(() => {
      setPage(root, { width: page.width, height: page.height, margins: { ...pageInfo(root).margins, [k]: Math.max(0, v) } });
    }, { coalesce: `margin:${k}`, props: false }), { min: "0" })));
  }
  form.append(section("report", t("props.report"), {},
    field(t("props.reportName"), h("input", { type: "text", value: root.attrs.name || "", oninput: (e) => change(() => { root.attrs.name = safeName(e.target.value); }, { coalesce: "name", props: false }) })),
    field(t("props.pageSize"), sizeSel),
    seg(t("props.orientation"), [["portrait", t("props.portrait")], ["landscape", t("props.landscape")]], page.landscape ? "landscape" : "portrait", (v) => change(() => {
      if ((v === "landscape") === page.landscape) return false;
      setPage(root, { width: page.height, height: page.width, margins: page.margins });
    })),
    h("div", { class: "prop" }, h("span", { class: "label" }, t("props.margins")), margins)));
  form.append(section("numbers", t("props.numbers"), {},
    field(t("props.locale"), h("select", { onchange: (e) => { S.locale = e.target.value; schedulePreview(); scheduleSave(); renderProps(); } },
      LOCALES.map((k) => h("option", { value: k, selected: k === S.locale }, t(`locale.${k}`)))), t("props.localeHelp", { sample: sampleOf("#,##0.00") }))));
  const grid = h("select", { onchange: (e) => { S.grid = Number(e.target.value); renderCanvas(); scheduleSave(); } }, [2, 5, 10].map((g) => h("option", { value: String(g), selected: g === S.grid }, t("props.points", { n: g }))));
  form.append(section("report-adv", t("props.advanced"), { fold: true },
    h("div", { class: "prop" }, check(t("props.autoHeading"), store.get(HEADINGS) !== "no", (v) => store.set(HEADINGS, v ? "yes" : "no"), { id: "auto-heading" })),
    field(t("props.grid"), grid, t("props.pointHelp"))));
}

function bandProps(form, b) {
  const nm = bandName(b);
  setTitle(nm.name, nm.where ? `${nm.where[0].toUpperCase()}${nm.where.slice(1)}.` : "");
  const hh = b.node ? num(b.node, "height") : 0;
  const minH = b.node ? elementsOf(b.node).reduce((m, el) => Math.max(m, num(el, "y") + num(el, "height")), 0) : 0;
  const empty = bandIsEmpty(b);
  form.append(section("band", t("props.thisArea"), {},
    h("p", { class: "help" }, t(empty ? "props.bandEmpty" : "props.bandFull")),
    field(t("props.bandHeight"), numberInput(hh, (v) => change(() => {
      const node = b.node || b.create();
      node.attrs.height = String(Math.max(minH, Math.round(v)));
    }, { coalesce: `band:${b.key}`, props: false }), { min: String(minH) }), empty ? t("props.heightEmpty") : t("props.heightHelp")),
    h("p", { class: "tech" }, t("band.tech", { name: nm.tech }))));
  if (!empty) {
    form.append(h("div", { class: "prop-actions" }, h("button", { type: "button", class: "danger", onclick: () => {
      change(() => { b.node.children = b.node.children.filter((c) => c.tag !== "element"); b.node.attrs.height = "0"; });
      announce(t("msg.bandEmptied", { band: nm.name }));
    } }, icon("trash"), t("props.emptyBand"))));
  }
}

/** An example of a number or date format, as the preview's language writes it. */
function sampleOf(pattern) {
  const loc = S.locale.replace("_", "-");
  if (!pattern) return t("fmt.asIs");
  if (/[yMdH]/.test(pattern)) {
    const d = new Date(2026, 10, 24, 14, 30);
    const month = (style) => new Intl.DateTimeFormat(loc, { month: style }).format(d).replace(/\.$/, "");
    return pattern.replace(/yyyy|MMMM|MMM|MM|dd|d|HH|mm/g, (k) => ({
      yyyy: "2026", MMMM: month("long"), MMM: month("short"), MM: "11", dd: "24", d: "24", HH: "14", mm: "30",
    })[k]);
  }
  const decimals = (/\.(0+)/.exec(pattern) || ["", ""])[1].length;
  if (pattern.endsWith("%")) return `${new Intl.NumberFormat(loc).format(12)}%`;
  const n = new Intl.NumberFormat(loc, { minimumFractionDigits: decimals, maximumFractionDigits: decimals, useGrouping: pattern.includes(",") }).format(1234.56);
  return pattern.startsWith("$") ? `$${n}` : n;
}

/** Choices for "Show only when": a yes/no field is yes or no, a field has a value. */
function whenChoices(el) {
  const { fields } = fieldsFor(el);
  const out = [];
  for (const f of fields) {
    if (f.type === "Yes/no") {
      out.push([`$F{${f.name}}`, t("when.yes", { name: f.name })]);
      out.push([`!$F{${f.name}}`, t("when.no", { name: f.name })]);
    }
  }
  for (const f of fields) if (f.type !== "Yes/no" && f.type !== "List") out.push([`$F{${f.name}} != null`, t("when.has", { name: f.name })]);
  return out;
}

function elementProps(form, el) {
  const kind = el.attrs.kind;
  const { parent } = index.get(el._id);
  const ed = (fn, key) => change(fn, { coalesce: key ? `${el._id}:${key}` : null, props: !key });
  const st = effectiveStyle(S.root, el);
  const isText = kind === "staticText" || kind === "textField";
  const expr = kind === "textField" ? (child(el, "expression") || {}).text || "" : "";
  const shownText = kind === "staticText" ? staticToText((child(el, "text") || {}).text || "") : kind === "textField" ? expressionToText(expr) : null;
  const isFormula = kind === "textField" && shownText === null;
  const isToday = kind === "textField" && expr.trim() === TODAY;
  setTitle(partName(el), t(`kind.${kind}.about`, {}, ""));

  if (isText) {
    const textSec = section("text", t("props.text"), {});
    if (isToday) {
      textSec.append(h("p", { class: "help strong" }, t("props.today")));
    } else if (isFormula) {
      textSec.append(h("p", { class: "formula-note" }, t("props.isFormula")));
    } else {
      const box = h("textarea", {
        id: "prop-text", rows: "2", spellcheck: "true", "aria-label": t("props.textBox"), "aria-autocomplete": "list", "aria-expanded": "false",
        "aria-describedby": "prop-text-help",
        oninput: (e) => {
          ed(() => { applyText(el, e.target.value); }, "text");
          pickerFollow(e.target);
          if (el.attrs.kind !== kind) {
            // It became a text with fields: the panel changes (number format...).
            const p = picker;
            renderProps();
            if (p) openPicker($("prop-text"), p.start);
          }
        },
        onkeydown: pickerKey,
        onblur: () => setTimeout(() => { if (picker && document.activeElement !== picker.box) closePicker(); }, 120),
      }, shownText);
      textSec.append(h("div", { class: "prop text-prop" }, box,
        h("div", { class: "text-tools" },
          h("button", { type: "button", class: "insert-btn", "aria-haspopup": "listbox", onclick: () => { box.focus(); openPicker(box, null); } }, h("span", { class: "pill" }, "+"), t("props.insertField")),
          h("span", { class: "help", id: "prop-text-help" }, t("props.textHelp")))));
    }
    if (kind === "textField") {
      const pat = el.attrs.pattern || "";
      const known = PATTERNS.includes(pat);
      const opt = (p) => h("option", { value: p, selected: p === pat }, sampleOf(p));
      const patSel = h("select", { onchange: (e) => ed(() => setAttr(el, "pattern", e.target.value === "custom" ? pat || "0.00" : e.target.value)) },
        opt(""), h("optgroup", { label: t("fmt.numbers") }, PATTERNS.slice(1, 5).map(opt)), h("optgroup", { label: t("fmt.dates") }, PATTERNS.slice(5).map(opt)),
        h("option", { value: "custom", selected: !known }, t("fmt.custom")));
      textSec.append(field(t("props.format"), patSel));
      if (!known) textSec.append(field(t("props.formatCode"), h("input", { type: "text", value: pat, spellcheck: "false", oninput: (e) => ed(() => setAttr(el, "pattern", e.target.value), "pattern") }), t("props.formatHelp")));
    }
    form.append(textSec);

    // Look
    const size = parseFloat(st.fontSize) || 10;
    const toggle = (attr, text, aria) => h("button", { type: "button", "aria-pressed": String(st[attr] === "true"), "aria-label": aria, title: aria, style: { fontWeight: attr === "bold" ? "800" : "650", fontStyle: attr === "italic" ? "italic" : "normal", textDecoration: attr === "underline" ? "underline" : "none", fontFamily: "Georgia, serif" },
      onclick: () => ed(() => setAttr(el, attr, st[attr] === "true" ? (el.attrs.style || kids(S.root, "style").some((s) => s.attrs.default === "true" && s.attrs[attr] === "true") ? "false" : null) : "true")) }, text);
    const filled = st.mode === "Opaque";
    form.append(section("look", t("props.look"), {},
      h("div", { class: "grid2 font-row" },
        h("label", {}, t("props.font"), h("select", { onchange: (e) => ed(() => setAttr(el, "fontName", e.target.value)) },
          ["DejaVu Sans", "DejaVu Serif", "DejaVu Sans Mono"].map((f) => h("option", { value: f, selected: (st.fontName || "DejaVu Sans") === f }, t(`font.${f}`))))),
        h("label", {}, t("props.size"), numberInput(size, (v) => ed(() => setAttr(el, "fontSize", Math.max(1, v)), "fontSize"), { min: "1", step: "0.5", "aria-label": t("props.fontSize") }))),
      h("div", { class: "prop" }, h("div", { class: "seg", role: "group", "aria-label": t("props.style") }, toggle("bold", "B", t("props.bold")), toggle("italic", "I", t("props.italic")), toggle("underline", "U", t("props.underline")))),
      h("div", { class: "colors" },
        h("label", {}, t("props.textColor"), colorInput(st.forecolor, (v) => ed(() => setAttr(el, "forecolor", v), "fore"), t("props.textColor"))),
        h("label", {}, t("props.background"), colorInput(st.backcolor || "#FFFFFF", (v) => ed(() => { setAttr(el, "backcolor", v); el.attrs.mode = "Opaque"; }, "back"), t("props.backgroundColor"))),
        check(t("props.noBackground"), !filled, (v) => ed(() => { if (v) { el.attrs.mode = "Transparent"; } else { el.attrs.mode = "Opaque"; if (!st.backcolor) el.attrs.backcolor = "#FFFFFF"; } })))));

    form.append(section("align", t("props.alignment"), {},
      h("div", { class: "align-row" },
        seg(t("props.across"), [["Left", icon("Left"), t("props.left")], ["Center", icon("Center"), t("props.center")], ["Right", icon("Right"), t("props.right")], ["Justified", icon("Justified"), t("props.justify")]], st.hTextAlign || "Left", (v) => ed(() => setAttr(el, "hTextAlign", v === "Left" ? null : v))),
        seg(t("props.upDown"), [["Top", icon("Top"), t("props.top")], ["Middle", icon("Middle"), t("props.middle")], ["Bottom", icon("Bottom"), t("props.bottom")]], st.vTextAlign || "Top", (v) => ed(() => setAttr(el, "vTextAlign", v === "Top" ? null : v))))));
    form.append(bordersSection(el, ed));
  }
  if (kind === "line" || kind === "rectangle" || kind === "ellipse") {
    const pen = readPen(el);
    const colors = h("div", { class: "colors" }, h("label", {}, kind === "line" ? t("props.color") : t("props.outline"), colorInput(st.forecolor, (v) => ed(() => setAttr(el, "forecolor", v), "fore"), kind === "line" ? t("props.lineColor") : t("props.outlineColor"))));
    if (kind !== "line") {
      const filled = st.mode !== "Transparent";
      colors.append(h("label", {}, t("props.fill"), colorInput(st.backcolor || "#FFFFFF", (v) => ed(() => { setAttr(el, "backcolor", v); delete el.attrs.mode; }, "back"), t("props.fillColor"))));
      colors.append(check(t("props.noFill"), !filled, (v) => ed(() => setAttr(el, "mode", v ? "Transparent" : null))));
    }
    const look = section("look", t("props.look"), {}, colors,
      h("div", { class: "grid2" },
        h("label", {}, t("props.thickness"), numberInput(pen.width, (v) => ed(() => writePen(el, { ...readPen(el), width: Math.max(0, v) }), "pen"), { min: "0", step: "0.25" })),
        h("label", {}, t("props.lineStyle"), h("select", { onchange: (e) => ed(() => writePen(el, { ...readPen(el), style: e.target.value })) },
          ["Solid", "Dashed", "Dotted", "Double"].map((s) => h("option", { value: s, selected: s === pen.style }, t(`line.${s}`)))))));
    if (kind === "rectangle") look.append(field(t("props.corners"), numberInput(num(el, "radius"), (v) => ed(() => setAttr(el, "radius", v > 0 ? Math.round(v) : null), "radius"), { min: "0" })));
    const g = geo(el);
    if (kind === "line" && g.w > 1 && g.h > 1) look.append(seg(t("props.direction"), [["TopDown", "↘", t("props.downRight")], ["BottomUp", "↗", t("props.upRight")]], el.attrs.direction || "TopDown", (v) => ed(() => setAttr(el, "direction", v === "TopDown" ? null : v))));
    form.append(look);
  }
  if (kind === "image") {
    const file = h("input", { type: "file", id: "prop-image-file", accept: "image/png,image/jpeg,image/svg+xml", hidden: true, onchange: async (e) => {
      const f = e.target.files[0];
      if (!f) return;
      if (f.size > 2 * 1024 * 1024) { announce(t("msg.imageTooBig"), "bad"); return; }
      const url = await new Promise((res) => { const r = new FileReader(); r.onload = () => res(r.result); r.readAsDataURL(f); });
      change(() => { setMainText(el, `"${url}"`); });
      announce(t("msg.imageAdded", { name: f.name }));
    } });
    form.append(section("picture", t("props.picture"), {},
      h("div", { class: "prop" }, h("div", { class: "row-btns" }, h("button", { type: "button", onclick: () => file.click() }, t("props.chooseImage")), file),
        h("span", { class: "help" }, t("props.imageHelp"))),
      field(t("props.fit"), h("select", { onchange: (e) => ed(() => setAttr(el, "scaleImage", e.target.value)) },
        [["RetainShape", t("props.fitInside")], ["FillFrame", t("props.fitStretch")], ["Clip", t("props.fitClip")]].map(([v, tx]) => h("option", { value: v, selected: (el.attrs.scaleImage || "RetainShape") === v }, tx))))));
  }
  if (kind === "frame") {
    const filled = st.mode === "Opaque";
    form.append(section("look", t("props.look"), {}, h("div", { class: "colors" },
      h("label", {}, t("props.background"), colorInput(st.backcolor || "#FFFFFF", (v) => ed(() => { setAttr(el, "backcolor", v); el.attrs.mode = "Opaque"; }, "back"), t("props.backgroundColor"))),
      check(t("props.noBackground"), !filled, (v) => ed(() => setAttr(el, "mode", v ? null : "Opaque"))))));
    form.append(bordersSection(el, ed));
  }
  if (!["staticText", "textField", "line", "rectangle", "ellipse", "image", "frame"].includes(kind)) {
    const inner = innerContainer(el);
    form.append(h("p", { class: "kept" }, inner ? t("props.listAbout", { name: listFieldOf(inner) || "" }) : t("props.cannotEdit")));
  }

  // Position and size
  const g = geo(el);
  const pos = h("div", { class: "grid4" });
  for (const [k, gk] of [["x", "x"], ["y", "y"], ["width", "w"], ["height", "h"]]) {
    pos.append(h("label", {}, t(`props.${k}`), numberInput(g[gk], (v) => ed(() => {
      setGeo(el, { [gk]: v });
      fit(el, parent);
    }, `geo:${k}`), { min: "0", "aria-label": t(`props.${k}Points`) })));
  }
  form.append(section("pos", t("props.position"), { fold: true }, h("div", { class: "prop" }, pos, h("span", { class: "help" }, t("props.positionHelp")))));

  // Advanced
  const adv = section("adv", t("props.advanced"), { fold: true, open: isFormula });
  if (kind === "textField") {
    adv.append(h("div", { class: "prop toggles" },
      check(t("props.grow"), el.attrs.textAdjust === "StretchHeight", (v) => ed(() => setAttr(el, "textAdjust", v ? "StretchHeight" : null))),
      check(t("props.blank"), el.attrs.blankWhenNull === "true", (v) => ed(() => setAttr(el, "blankWhenNull", v ? "true" : null)))));
  }
  const pw = child(el, "printWhenExpression");
  const pwText = pw ? (pw.text || "").trim() : "";
  const choices = whenChoices(el);
  const matched = S.whenCustom === el._id ? "custom" : !pwText ? "" : choices.some(([v]) => v === pwText) ? pwText : "custom";
  const setWhen = (value) => {
    let n = child(el, "printWhenExpression");
    if (!value.trim()) { if (n) el.children.splice(el.children.indexOf(n), 1); return; }
    if (!n) { n = make("printWhenExpression", {}, [], ""); el.children.unshift(n); }
    n.text = value;
  };
  adv.append(field(t("props.showWhen"), h("select", { onchange: (e) => {
    const v = e.target.value;
    S.whenCustom = v === "custom" ? el._id : null;
    if (v === "custom") { S.open.add("adv"); renderProps(); return; }
    ed(() => setWhen(v));
  } }, h("option", { value: "", selected: matched === "" }, t("when.always")), choices.map(([v, tx]) => h("option", { value: v, selected: matched === v }, tx)),
    h("option", { value: "custom", selected: matched === "custom" }, t("when.custom")))));
  if (matched === "custom") {
    adv.append(field(t("props.whenFormula"), h("input", { type: "text", value: pwText, spellcheck: "false", class: "code-input", oninput: (e) => ed(() => setWhen(e.target.value), "printWhen") }), t("props.whenHelp")));
  }
  if (kind === "textField") {
    adv.append(h("div", { class: `prop formula${isFormula ? " is-formula" : ""}` },
      h("label", { for: "prop-expr" }, t("props.formula")),
      h("textarea", { id: "prop-expr", rows: isFormula ? "4" : "2", spellcheck: "false", class: "code-input", oninput: (e) => ed(() => { setMainText(el, e.target.value); }, "expr"), onchange: () => renderProps() }, expr),
      h("span", { class: "help" }, t(isFormula ? "props.formulaHelpFx" : "props.formulaHelp"))));
  }
  form.append(adv);

  form.append(h("div", { class: "prop-actions" },
    h("button", { type: "button", onclick: duplicateSelected }, icon("copy"), t("props.duplicate")),
    h("button", { type: "button", onclick: () => reorder(true) }, icon("front"), t("props.toFront")),
    h("button", { type: "button", onclick: () => reorder(false) }, icon("back"), t("props.toBack")),
    h("button", { type: "button", class: "danger", onclick: removeSelected }, icon("trash"), t("props.delete"))));
}

function bordersSection(el, ed) {
  const b = readBorders(el);
  const any = Object.values(b.sides).some(Boolean);
  const patchKey = (p) => (p.color ? "borderColor" : null);
  const write = (patch) => ed(() => {
    const cur = readBorders(el);
    const next = { ...cur, ...patch, sides: { ...cur.sides, ...(patch.sides || {}) } };
    if (!Object.values(cur.sides).some(Boolean) && !patch.sides && patch.width) next.sides = { top: true, left: true, bottom: true, right: true };
    if (next.width === 0) next.width = 0.5;
    writeBorders(el, next);
  }, patchKey(patch));
  const sides = h("div", { class: "seg", role: "group", "aria-label": t("props.sides") },
    [["top", t("props.top")], ["bottom", t("props.bottom")], ["left", t("props.left")], ["right", t("props.right")]].map(([s, tx]) => h("button", { type: "button", "aria-pressed": String(b.sides[s]), "aria-label": t("props.sideBorder", { side: tx }), title: t("props.sideBorder", { side: tx }), onclick: () => write({ sides: { [s]: !b.sides[s] } }) }, tx)));
  return section("border", t("props.border"), { fold: true, open: any },
    h("div", { class: "prop" }, h("span", { class: "label" }, t("props.sides")), sides),
    h("div", { class: "colors" },
      h("label", {}, t("props.width"), h("select", { onchange: (e) => { const w = Number(e.target.value); if (w === 0) ed(() => writeBorders(el, { ...readBorders(el), width: 0 })); else write({ width: w }); } },
        [[0, t("border.none")], [0.5, t("border.thin")], [1, "1 pt"], [2, "2 pt"], [3, "3 pt"]].map(([v, tx]) => h("option", { value: String(v), selected: any ? Math.abs(b.width - v) < 0.01 : v === 0 }, tx)))),
      h("label", {}, t("props.color"), colorInput(b.color, (v) => write({ color: v }), t("props.borderColor"))),
      h("label", {}, t("props.padding"), numberInput(num(child(el, "box") || { attrs: {} }, "padding"), (v) => ed(() => {
        let box = child(el, "box");
        if (!box) { box = make("box"); el.children.push(box); }
        setAttr(box, "padding", v > 0 ? Math.round(v) : null);
        if (!box.children.length && !Object.keys(box.attrs).length) el.children.splice(el.children.indexOf(box), 1);
      }, "padding"), { min: "0", "aria-label": t("props.paddingPoints") }))));
}

// ---------------------------------------------------------------------------------------------
// Preview and export

let previewTimer = null;
function schedulePreview() {
  clearTimeout(previewTimer);
  previewTimer = setTimeout(renderPreview, 400);
}

const templateText = (to = "jrxml") => engine.fromTree(toWire(S.root), to);

function renderTo(format) {
  const jrxml = templateText("jrxml");
  return engine.render(jrxml, S.rows, S.params, { format, locale: S.locale, maxPages: 500, resources: S.resources });
}

let previewState = { ok: true, text: "" };
function renderPreview() {
  if (!engine) return;
  const errBox = $("preview-error");
  try {
    const t0 = performance.now();
    const pdf = renderTo("pdf");
    const ms = performance.now() - t0;
    const url = URL.createObjectURL(new Blob([pdf], { type: "application/pdf" }));
    const old = previewUrl;
    previewUrl = url;
    if (old && old !== previewShown) URL.revokeObjectURL(old);
    errBox.hidden = true;
    previewState = { ok: true, ms, kb: pdf.length / 1024 };
    document.body.dataset.previewBytes = String(pdf.length);
    document.body.dataset.previewCount = String(Number(document.body.dataset.previewCount || 0) + 1);
    showPreview();
  } catch (e) {
    previewState = { ok: false };
    errBox.hidden = false;
    const text = String(e.message || e);
    const missing = /`([^`]+)` not found in the resource directories/.exec(text);
    errBox.textContent = missing ? t("preview.missingFile", { file: missing[1] })
      : /null used as a boolean|on null/.test(text) && !S.rows.length ? `${text}\n\n${t("preview.needsData")}` : text;
  }
  renderPreviewMsg();
}
function renderPreviewMsg() {
  const msg = $("preview-msg");
  if (!engine) return;
  if (previewState.ok) {
    msg.textContent = t("preview.upToDate", { ms: previewState.ms < 10 ? previewState.ms.toFixed(1) : Math.round(previewState.ms), kb: previewState.kb.toFixed(0) });
    msg.className = "msg good";
  } else {
    msg.textContent = t("preview.problem");
    msg.className = "msg bad";
  }
  $("tab-alert").hidden = previewState.ok;
}
/** Puts the latest PDF in the preview, when it is on screen. */
function showPreview() {
  if (S.view !== "preview" || !previewUrl || previewShown === previewUrl) return;
  const old = previewShown;
  previewShown = previewUrl;
  $("preview").src = `${previewUrl}#toolbar=0&navpanes=0&view=FitH`;
  if (old) setTimeout(() => URL.revokeObjectURL(old), 4000);
}

function setView(view) {
  S.view = view;
  const preview = view === "preview";
  $("tab-design").setAttribute("aria-selected", String(!preview));
  $("tab-preview").setAttribute("aria-selected", String(preview));
  $("tab-design").tabIndex = preview ? -1 : 0;
  $("tab-preview").tabIndex = preview ? 0 : -1;
  $("canvas-stage").hidden = preview;
  $("preview-pane").hidden = !preview;
  document.body.classList.toggle("previewing", preview);
  if (preview) {
    if (!previewUrl) renderPreview();
    showPreview();
  }
}

function download(bytes, name, type) {
  const url = URL.createObjectURL(new Blob([bytes], { type }));
  const a = h("a", { href: url, download: name });
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 5000);
}

const baseName = () => safeName(S.root.attrs.name || S.name).replace(/\s+/g, "-").toLowerCase() || "report";

function saveTemplate(to) {
  closeMenus();
  try {
    const text = templateText(to);
    download(text, `${baseName()}.${to}`, to === "jrxml" ? "application/xml" : "text/plain");
    announce(t("msg.saved", { file: `${baseName()}.${to}` }));
  } catch (e) {
    announce(t("msg.cannotSave", { error: e.message || e }), "bad");
  }
}

function exportReport(format) {
  closeMenus();
  const types = { pdf: "application/pdf", xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document" };
  try {
    download(renderTo(format), `${baseName()}.${format}`, types[format]);
    announce(t("msg.downloaded", { file: `${baseName()}.${format}` }));
  } catch (e) {
    announce(t("msg.cannotMake", { format: format.toUpperCase(), error: e.message || e }), "bad");
  }
}

// ---------------------------------------------------------------------------------------------
// Opening, starting and keeping work

/** Reads a template (and the files it imports) into the designer. */
function openTemplate(text, resources = {}, name = "report") {
  const tree = engine.toTree(text, { resources });
  S.resources = resources;
  if (tree.tag !== "jasperReport") throw new Error(t("msg.styleTemplate"));
  S.root = fromWire(tree);
  S.name = name;
  S.sel = null;
  S.undo = [];
  S.redo = [];
  S.coalesce = null;
}

async function startWith(kind) {
  closeMenus();
  try {
    if (kind === "blank") {
      S.root = blankReport(t("start.blankName"));
      S.name = "report";
      S.resources = {};
      setParams("{}", false);
      S.sel = null; S.undo = []; S.redo = [];
      setData(`[\n  {"name": "Ana Peña", "city": "Santo Domingo", "amount": 1250.5},\n  {"name": "Luis Mejía", "city": "Santiago", "amount": 980}\n]`, false);
    } else {
      const files = kind === "invoice" ? ["invoices.ferro", "parts.ferro", "invoices.json"] : ["employees.ferro", null, "employees.json"];
      const get = async (f) => { const r = await fetch(`examples/${f}`); if (!r.ok) throw new Error(`examples/${f}: ${r.status}`); return r.text(); };
      const [tpl, parts, data] = await Promise.all([get(files[0]), files[1] ? get(files[1]) : null, get(files[2])]);
      openTemplate(tpl, parts ? { [files[1]]: parts } : {}, files[0].replace(/\.ferro$/, ""));
      setParams("{}", false);
      setData(JSON.stringify(JSON.parse(data), null, 2), false);
    }
    setView("design");
    fitZoom();
    refresh();
    announce(kind === "blank" ? t("msg.blank") : t(kind === "invoice" ? "msg.openedInvoice" : "msg.openedEmployees"));
  } catch (e) {
    announce(t("msg.cannotOpen", { error: e.message || e }), "bad");
  }
}

async function openFiles(fileList) {
  const files = [...fileList];
  const isText = (n) => /\.(ferro|jrxml|xml|jrtx|json)$/i.test(n);
  const read = await Promise.all(files.map(async (f) => ({ name: f.name, text: isText(f.name) ? await f.text() : null, bytes: isText(f.name) ? null : new Uint8Array(await f.arrayBuffer()) })));
  // JSON: an object named like params.json holds parameter values; anything else is the data.
  const jsons = read.filter((f) => /\.json$/i.test(f.name));
  const paramFile = jsons.find((f) => /param/i.test(f.name));
  const data = jsons.find((f) => f !== paramFile);
  const templates = read.filter((f) => f.text !== null && !/\.json$/i.test(f.name));
  const main = templates.find((f) => /^\s*(\/\/.*\n\s*)*report\b/.test(f.text) || /<jasperReport\b/.test(f.text)) || templates[0];
  try {
    if (main) {
      // The other files are what it uses: imports, subreports, style templates, images.
      const resources = Object.fromEntries(read.filter((f) => f !== main && !/\.json$/i.test(f.name)).map((f) => [f.name, f.text !== null ? f.text : f.bytes]));
      openTemplate(main.text, resources, main.name.replace(/\.(ferro|jrxml|xml)$/i, ""));
      setParams(paramFile ? paramFile.text : "{}", false);
    } else if (paramFile) setParams(paramFile.text, false);
    let madeUp = false;
    if (data) setData(data.text, false);
    else if (main) {
      // Data for other fields than the template's would leave its parts empty: make some up.
      const names = declaredFields(S.root).map((f) => f.name);
      const have = new Set(S.dataFields.map((f) => f.name));
      if (names.length && !names.some((n) => have.has(n))) {
        setData(JSON.stringify(sampleFor(S.root), null, 2), false);
        madeUp = true;
      }
    }
    setView("design");
    fitZoom();
    refresh();
    announce(main ? t(data ? "msg.openedWithData" : madeUp ? "msg.openedMadeUp" : "msg.opened", { file: main.name, data: data && data.name }) : t("msg.dataFrom", { file: data.name }));
  } catch (e) {
    const msg = String(e.message || e);
    announce(/import/.test(msg) ? t("msg.needsImports", { file: main.name }) : t("msg.cannotOpenFile", { file: main ? main.name : "", error: msg }), "bad");
  }
}

let saveTimer = null;
function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    store.set(STORE, JSON.stringify({ tree: toWire(S.root), data: S.dataText, params: S.paramsText, name: S.name, locale: S.locale, grid: S.grid, snap: S.snap, zoom: S.zoom }));
  }, 300);
}

function closeMenus() {
  document.querySelectorAll("details.menu[open]").forEach((d) => d.removeAttribute("open"));
}

/** The zoom at which the page fills the width there is for it. */
function fitZoom() {
  const stage = $("canvas-stage");
  const avail = (stage.clientWidth || $("center").clientWidth) - gutter() - 40;
  const w = pageInfo(S.root).width * PT;
  S.zoom = Math.max(0.25, Math.min(1.6, Math.floor((avail / w) * 100) / 100));
  S.autoZoom = true;
}
function setZoom(z) {
  S.zoom = Math.max(0.25, Math.min(4, Math.round(z * 100) / 100));
  S.autoZoom = false;
  renderCanvas();
  scheduleSave();
}

function openGuide() {
  closeMenus();
  const d = $("guide");
  if (!d.open) d.showModal();
}

function changeLang(l) {
  setLang(l);
  document.querySelectorAll("[data-lang]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.lang === lang())));
  renderDataMsg();
  renderPreviewMsg();
  refresh();
}

// ---------------------------------------------------------------------------------------------
// Wiring

function wire() {
  const stage = $("canvas-stage");
  stage.addEventListener("pointerdown", startPointer);
  stage.addEventListener("dragover", onDragOver);
  stage.addEventListener("dragleave", (e) => { if (!stage.contains(e.relatedTarget)) clearDrop(); });
  stage.addEventListener("drop", onDrop);
  stage.addEventListener("focusin", (e) => {
    const elDiv = e.target.closest(".el");
    if (elDiv) {
      const id = Number(elDiv.dataset.id);
      if (!(S.sel && S.sel.type === "el" && S.sel.id === id)) select({ type: "el", id }, { focus: true });
    }
  });
  stage.addEventListener("click", (e) => {
    const lbl = e.target.closest("[data-select-band]");
    if (lbl && e.detail === 0) select({ type: "band", key: lbl.dataset.selectBand }, { focus: true }); // keyboard
  });
  // Double-click a text to change what it says.
  stage.addEventListener("dblclick", (e) => {
    if (!e.target.closest(".el")) return;
    const box = $("prop-text") || $("prop-expr");
    if (box) { box.focus(); box.select(); }
  });
  document.addEventListener("keydown", onKey);
  stage.addEventListener("wheel", (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    e.preventDefault();
    setZoom(S.zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1));
  }, { passive: false });

  document.querySelectorAll("#parts button").forEach((b) => {
    const payload = { type: "part", part: b.dataset.part };
    b.addEventListener("dragstart", (e) => startDrag(e, payload));
    b.addEventListener("dragend", () => { dragPayload = null; clearDrop(); });
    b.addEventListener("click", () => addThing(payload, defaultTarget(), null));
  });

  let dataTimer;
  $("data").addEventListener("input", (e) => { clearTimeout(dataTimer); dataTimer = setTimeout(() => setData(e.target.value), 350); });
  let paramsTimer;
  $("params").addEventListener("input", (e) => { clearTimeout(paramsTimer); paramsTimer = setTimeout(() => setParams(e.target.value), 350); });
  $("data-upload-btn").addEventListener("click", () => $("data-file").click());
  $("data-file").addEventListener("change", async (e) => {
    const f = e.target.files[0];
    if (f) { setData(await f.text()); announce(t("msg.dataFrom", { file: f.name })); }
    e.target.value = "";
  });

  document.querySelectorAll("[data-start]").forEach((b) => b.addEventListener("click", () => {
    if ($("guide").open) $("guide").close();
    startWith(b.dataset.start);
  }));
  const openFile = () => { closeMenus(); if ($("guide").open) $("guide").close(); $("open-file").click(); };
  $("open-btn").addEventListener("click", openFile);
  $("guide-open").addEventListener("click", openFile);
  $("open-file").addEventListener("change", (e) => { if (e.target.files.length) openFiles(e.target.files); e.target.value = ""; });
  $("save-ferro").addEventListener("click", () => saveTemplate("ferro"));
  $("save-jrxml").addEventListener("click", () => saveTemplate("jrxml"));
  $("download-pdf").addEventListener("click", () => exportReport("pdf"));
  document.querySelectorAll("[data-export]").forEach((b) => b.addEventListener("click", () => exportReport(b.dataset.export)));
  document.addEventListener("click", (e) => { document.querySelectorAll("details.menu[open]").forEach((d) => { if (!d.contains(e.target)) d.removeAttribute("open"); }); });
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    document.querySelectorAll("details.menu[open]").forEach((d) => { d.removeAttribute("open"); d.querySelector("summary").focus(); });
  });
  document.querySelectorAll("details.menu").forEach((d) => d.addEventListener("toggle", () => {
    if (d.open) document.querySelectorAll("details.menu[open]").forEach((o) => { if (o !== d) o.removeAttribute("open"); });
  }));
  // Menus: arrow keys go from item to item.
  document.querySelectorAll(".menu-list").forEach((list) => list.addEventListener("keydown", (e) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...list.querySelectorAll("button, input")];
    const i = items.indexOf(document.activeElement);
    items[(i + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length].focus();
  }));

  $("undo").addEventListener("click", undo);
  $("redo").addEventListener("click", redo);
  $("zoom-in").addEventListener("click", () => setZoom(S.zoom * 1.25));
  $("zoom-out").addEventListener("click", () => setZoom(S.zoom / 1.25));
  $("zoom-label").addEventListener("click", () => { fitZoom(); renderCanvas(); scheduleSave(); });
  $("snap").addEventListener("change", (e) => { S.snap = e.target.checked; renderCanvas(); scheduleSave(); });
  $("tab-design").addEventListener("click", () => setView("design"));
  $("tab-preview").addEventListener("click", () => setView("preview"));
  $("views").addEventListener("keydown", (e) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    setView(S.view === "design" ? "preview" : "design");
    $(S.view === "design" ? "tab-design" : "tab-preview").focus();
  });
  $("help-btn").addEventListener("click", () => { closeMenus(); $("help").showModal(); });
  $("guide-btn").addEventListener("click", openGuide);
  $("guide").addEventListener("close", () => store.set(WELCOMED, "yes"));
  document.querySelectorAll("[data-lang]").forEach((b) => b.addEventListener("click", () => changeLang(b.dataset.lang)));
  let resizeTimer;
  window.addEventListener("resize", () => { clearTimeout(resizeTimer); resizeTimer = setTimeout(() => { if (S.autoZoom) { fitZoom(); renderCanvas(); } }, 150); });
}

async function boot() {
  applyStatic();
  document.querySelectorAll("[data-lang]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.lang === lang())));
  wire();
  const firstTime = store.get(WELCOMED) !== "yes";
  try {
    engine = await import("../pkg/ferroreport.js");
    await engine.default();
  } catch (e) {
    $("preview-msg").textContent = t("preview.noEngine");
    $("preview-msg").className = "msg bad";
    $("preview-error").hidden = false;
    $("preview-error").textContent = t("preview.noEngineLong", { error: e.message || e });
    setView("preview");
    return;
  }
  const params = new URLSearchParams(location.search);
  const start = params.get("start");
  let saved = null;
  try { saved = JSON.parse(store.get(STORE) || "null"); } catch { saved = null; }
  let restored = false;
  if (!start && saved && saved.tree) {
    try {
      S.root = fromWire(saved.tree);
      S.name = saved.name || "report";
      S.locale = saved.locale || "en_US";
      S.grid = saved.grid || 5;
      S.snap = saved.snap !== false;
      $("snap").checked = S.snap;
      setData(saved.data || "[]", false);
      setParams(saved.params || "{}", false);
      fitZoom();
      refresh();
      announce(t("msg.restored"));
      restored = true;
    } catch { /* fall through to the example */ }
  }
  if (!restored) await startWith(["blank", "invoice", "employees"].includes(start) ? start : "invoice");
  if (firstTime) openGuide();
}

// For the browser console and automated tests: the template being designed.
window.designer = {
  tree: () => toWire(S.root),
  template: (to = "jrxml") => templateText(to),
  rows: () => S.rows,
  lang: () => lang(),
};

boot();
