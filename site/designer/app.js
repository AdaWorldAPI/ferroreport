// The report designer: a page with the report's bands, parts placed on them, a properties
// panel, and a live preview made by the ferroreport engine (WebAssembly) in the browser.
// Templates are read and written by the engine (toTree / fromTree); this file only edits the
// tree in between.
import {
  fromWire, toWire, reId, make, child, kids, num, setAttr, round, insertOrdered, bands, elementsOf, innerContainer,
  indexTree, datasetOfContainer, elementLabel, elementText, setMainText, newElement, readBorders, writeBorders,
  readPen, writePen, effectiveStyle, detectFields, sampleRows, declaredFields, ensureField, safeName, PAGE_SIZES,
  pageInfo, setPage, blankReport, sampleFor, CLASS_TYPES,
} from "./model.js";

const $ = (id) => document.getElementById(id);
const PT = 4 / 3; // CSS pixels in a point: at 100% the page is its real size
const GUTTER = 110; // room for the band names, left of the page
const EMPTY_BAND = 18; // how tall (in screen pixels) an empty band is drawn, so there is somewhere to drop
const STORE = "ferroreport.designer.v1";
const LOCALES = { en_US: "English (US)", en_GB: "English (UK)", es_DO: "Spanish (Dominican Republic)", es_MX: "Spanish (Mexico)", es_ES: "Spanish (Spain)", fr_FR: "French", de_DE: "German", pt_BR: "Portuguese (Brazil)" };
const PATTERNS = [
  ["", "As it is"], ["#,##0.00", "1,234.56"], ["#,##0", "1,234"], ["$#,##0.00", "$1,234.56"], ["0%", "12%"],
  ["MMM d, yyyy", "Jan 5, 2026"], ["dd/MM/yyyy", "05/01/2026"], ["yyyy-MM-dd", "2026-01-05"], ["MMMM d, yyyy HH:mm", "January 5, 2026 14:30"],
];

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
  preview: true,
};
let engine = null;
let index = new Map();
let bandList = [];
let dragPayload = null;
let previewUrl = null;

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
  announce("Undone");
}
function redo() {
  if (!S.redo.length) return;
  S.undo.push(snapshot());
  restore(S.redo.pop());
  S.coalesce = null;
  if (S.sel && S.sel.type === "el" && !indexTree(S.root).has(S.sel.id)) S.sel = null;
  refresh();
  announce("Redone");
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
  let t = document.querySelector(".toast");
  if (!t) {
    t = h("div", { class: "toast", "aria-hidden": "true" });
    document.body.append(t);
  }
  t.textContent = text;
  t.className = `toast show ${kind}`;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove("show"), kind === "bad" ? 6000 : 2600);
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
    const shown = hh > 0 ? hh : EMPTY_BAND / sc;
    const row = { b, y, h: hh, shown };
    y += shown;
    return row;
  });
  const pageH = Math.max(y + page.margins.bottom, 120);
  sheet.style.width = `${GUTTER + page.width * sc}px`;

  const pageEl = h("div", { class: "page", style: { left: `${GUTTER}px`, width: `${page.width}px`, height: `${pageH}px`, transform: `scale(${sc})` } });
  pageEl.append(h("div", { class: "margin-guide", style: { left: `${page.margins.left}px`, top: `${page.margins.top}px`, width: `${page.columnWidth}px`, height: `${pageH - page.margins.top - page.margins.bottom}px` } }));
  sheet.append(pageEl);

  let labelBottom = 0;
  for (const row of rows) {
    const { b } = row;
    const selected = S.sel && S.sel.type === "band" && S.sel.key === b.key;
    const bandEl = h("div", {
      class: `band${b.node && row.h > 0 ? "" : " empty-band"}${S.snap ? " grid" : ""}${selected ? " selected" : ""}`,
      style: { left: `${page.margins.left}px`, top: `${row.y}px`, width: `${page.columnWidth}px`, height: `${row.shown}px`, backgroundSize: `${S.grid}px ${S.grid}px` },
      dataset: { bandKey: b.key, ...(b.node ? { cid: String(b.node._id) } : {}) },
    });
    if (b.node) renderElements(b.node, bandEl);
    const resize = h("div", { class: "band-resize", title: `Drag to change the height of ${b.label}`, dataset: { resizeBand: b.key } });
    bandEl.append(resize);
    pageEl.append(bandEl);

    // Names of thin bands would overlap: each starts below the one before.
    const top = Math.max(row.y * sc, labelBottom);
    const room = Math.max(row.shown * sc - (top - row.y * sc), 16);
    labelBottom = top + room;
    const label = h("button", {
      type: "button", class: `band-label${selected ? " selected" : ""}`,
      style: { top: `${top}px`, height: `${room}px` },
      dataset: { selectBand: b.key },
      "aria-label": `${b.label} band, ${row.h} points tall. ${b.help}.`,
      "aria-pressed": selected ? "true" : "false",
    }, h("b", {}, b.label), room >= 40 ? h("small", {}, b.help) : null);
    sheet.append(label);
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

function renderElement(el) {
  const g = geo(el);
  const kind = el.attrs.kind;
  const selected = S.sel && S.sel.type === "el" && S.sel.id === el._id;
  const div = h("div", {
    class: `el${selected ? " selected" : ""}`, tabindex: "0", role: "button",
    "aria-label": `${elementLabel(el)}${elementText(el) ? `: ${elementText(el)}` : ""}, at ${g.x}, ${g.y}, ${g.w} by ${g.h} points`,
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
      const expr = (child(el, "expression") || {}).text || "";
      // Field names stand out; the rest of an expression is shown as written.
      const parts = expr.split(/(\$[FPV]\{[^}]+\})/);
      for (const p of parts) {
        const m = /^\$([FPV])\{([^}]+)\}$/.exec(p);
        if (m) span.append(h("span", { class: "chip" }, m[2] === "PAGE_NUMBER" ? "page" : m[2]));
        else if (p) span.append(p);
      }
      if (!expr) span.append(h("span", { class: "chip" }, "empty"));
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
    else div.append(expr ? expr.replace(/^"|"$/g, "") : "Image: choose a file");
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
      div.append(h("span", { class: "el-tag" }, elementLabel(el)));
    } else {
      div.classList.add("other");
      div.append(h("span", { class: "el-tag" }, elementLabel(el)));
    }
    if (opaque && !inner) div.style.background = st.backcolor;
  }
  if (selected) {
    const hs = 7 / scale(); // handles stay the same size on screen
    for (const [k, l, t] of [["nw", 0, 0], ["n", .5, 0], ["ne", 1, 0], ["e", 1, .5], ["se", 1, 1], ["s", .5, 1], ["sw", 0, 1], ["w", 0, .5]]) {
      if (kind === "line" && g.h <= 1 && !["e", "w"].includes(k)) continue;
      if (kind === "line" && g.w <= 1 && g.h > 1 && !["n", "s"].includes(k)) continue;
      div.append(h("div", { class: "handle", dataset: { h: k }, style: { width: `${hs}px`, height: `${hs}px`, left: `calc(${l * 100}% - ${hs / 2}px)`, top: `calc(${t * 100}% - ${hs / 2}px)` } }));
    }
  }
  return div;
}

// ---------------------------------------------------------------------------------------------
// Selecting, moving and resizing with the mouse

function select(sel, { focus = false } = {}) {
  S.sel = sel;
  S.coalesce = null;
  renderCanvas();
  renderProps();
  if (focus) {
    const t = sel && sel.type === "el" ? $("sheet").querySelector(`[data-id="${sel.id}"]`) : sel && sel.type === "band" ? $("sheet").querySelector(`[data-select-band="${CSS.escape(sel.key)}"]`) : null;
    if (t) t.focus({ preventScroll: true });
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
    bandDiv.style.height = `${Math.max(hh, EMPTY_BAND / scale())}px`;
  }, () => {
    handle.classList.remove("active");
    if (hh === startH) return;
    S.sel = { type: "band", key };
    change(() => {
      const node = b.node || b.create();
      node.attrs.height = String(hh);
    });
    announce(`${b.label}: ${hh} points tall`);
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
    announce(e.shiftKey ? `${g.w} by ${g.h} points` : `At ${g.x}, ${g.y}`);
  }
}

function removeSelected() {
  if (!S.sel || S.sel.type !== "el") return;
  const { node, parent } = index.get(S.sel.id);
  const label = elementLabel(node);
  S.sel = null;
  change(() => { parent.children.splice(parent.children.indexOf(node), 1); });
  announce(`${label} removed. Ctrl+Z brings it back.`);
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
  announce("Duplicated");
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

/** Where a part goes when it is added without the mouse: the selected band, or the detail band. */
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
      const t = $("prop-text");
      if (t) { t.focus(); t.select(); }
    }
    if (payload.type === "part" && payload.part === "image") $("prop-image-file")?.click();
  }
  return added;
}

function place(container, w, hh, at) {
  const p = at ? { x: snapV(at.x), y: snapV(at.y) } : freeSpot(container, w, hh);
  return p;
}

function addField(f, container, at) {
  const ds = datasetOfContainer(S.root, container, index);
  if (f.param) {
    if (ds) { announce("Parameters of the report can't be shown inside a list yet: put it outside the list.", "bad"); return null; }
    const p = place(container, 140, 20, at);
    const el = newElement("textField", p.x, p.y, 140, 20, { blankWhenNull: "true" });
    setMainText(el, `$P{${f.name}}`);
    container.children.push(el);
    index = indexTree(S.root);
    fit(el, container);
    return el;
  }
  if (f.type === "List") {
    if (ds) { announce("A list inside a list is not supported yet: drop it on a band.", "bad"); return null; }
    return addList(f, container, at);
  }
  if (f.list && !ds) {
    const hasList = [...index.values()].some((x) => x.node.tag === "contents" && listFieldOf(x.node) === f.list);
    announce(hasList
      ? `“${f.name}” is part of each item of “${f.list}”: put it inside the list of “${f.list}” on the page.`
      : `“${f.name}” is part of each item of “${f.list}”. Drag “${f.list}” onto the page first, then put “${f.name}” in the list it makes.`, "bad");
    return null;
  }
  if (!f.list && ds) {
    announce(`This list repeats for each item of its data: put fields of those items here, and “${f.name}” outside the list.`, "bad");
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
  // A field dropped in the detail band gets its heading in the column header, above it.
  const detailBand = bandList.find((b) => b.kind === "detail" && b.node === container);
  if (localStorage.getItem("ferroreport.designer.headings") !== "no") {
    if (detailBand) {
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
  announce(`Added a list that repeats for each item of “${f.name}”`);
  return list;
}

function addPart(part, container, at) {
  const sizes = { staticText: [120, 20], line: [200, 1], rectangle: [120, 60], ellipse: [60, 60], image: [80, 60], pageNumber: [100, 20], today: [110, 20] };
  const [w, hh] = sizes[part];
  const p = place(container, w, hh, at);
  let el;
  if (part === "staticText") {
    el = newElement("staticText", p.x, p.y, w, hh);
    setMainText(el, "Text");
  } else if (part === "pageNumber") {
    el = newElement("textField", p.x, p.y, w, hh, { hTextAlign: "Right" });
    setMainText(el, `"Page " + $V{PAGE_NUMBER}`);
  } else if (part === "today") {
    el = newElement("textField", p.x, p.y, w, hh, { pattern: "MMM d, yyyy" });
    setMainText(el, "new java.util.Date()");
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
  if (payload.type === "part") return { staticText: [120, 20], line: [200, 1], rectangle: [120, 60], ellipse: [60, 60], image: [80, 60], pageNumber: [100, 20], today: [110, 20] }[payload.part];
  const t = payload.field.type;
  return t === "List" ? [200, 20] : t === "Number" || t === "Whole number" || t === "Date" ? [90, 20] : [140, 20];
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
  const holder = target.querySelector(":scope > .contents") || target;
  const r = holder.getBoundingClientRect();
  const [w, hh] = dragSize(dragPayload);
  const x = snapV((e.clientX - r.left) / scale() - Math.min(w, 40) / 2), y = snapV((e.clientY - r.top) / scale() - hh / 2);
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
  const msg = $("data-msg");
  try {
    const parsed = text.trim() ? JSON.parse(text) : [];
    S.rows = sampleRows(parsed);
    S.dataFields = detectFields(S.rows);
    msg.textContent = `${S.rows.length} ${S.rows.length === 1 ? "row" : "rows"}, ${S.dataFields.length} ${S.dataFields.length === 1 ? "field" : "fields"}`;
    msg.className = "msg good";
  } catch (err) {
    msg.textContent = `Not valid JSON yet: ${err.message.replace(/^JSON\.parse: /, "")}`;
    msg.className = "msg bad";
    return;
  }
  if (record) {
    renderFields();
    schedulePreview();
    scheduleSave();
  }
}

function setParams(text, record = true) {
  S.paramsText = text;
  if ($("params").value !== text) $("params").value = text;
  const msg = $("params-msg");
  try {
    const v = text.trim() ? JSON.parse(text) : {};
    if (!v || typeof v !== "object" || Array.isArray(v)) throw new Error("write an object, as in {\"CUSTOMER\": \"Caribe\"}");
    S.params = v;
    msg.textContent = "";
    msg.className = "msg";
  } catch (err) {
    msg.textContent = `Not valid yet: ${err.message}`;
    msg.className = "msg bad";
    return;
  }
  if (record) {
    schedulePreview();
    scheduleSave();
  }
}

function renderParams() {
  const params = kids(S.root, "parameter").map((p) => ({ name: p.attrs.name, class: p.attrs.class || "java.lang.String", type: CLASS_TYPES[p.attrs.class] || "Text", param: true }));
  $("params-section").hidden = params.length === 0;
  const ul = $("param-list");
  ul.textContent = "";
  for (const f of params) {
    const payload = { type: "field", field: f };
    ul.append(h("li", {}, h("button", {
      type: "button", class: "field-btn", draggable: "true", title: "Drag onto the page, or press Enter to add it to the selected band",
      "aria-label": `Parameter ${f.name}, ${f.type}. Add to the page`,
      ondragstart: (e) => startDrag(e, payload), ondragend: () => { dragPayload = null; clearDrop(); },
      onclick: () => addThing(payload, defaultTarget(), null),
    }, h("span", { class: "name" }, f.name), h("span", { class: "type", dataset: { t: f.type } }, f.type))));
  }
  const names = Object.keys(S.resources);
  const note = $("files-note");
  note.hidden = names.length === 0;
  note.textContent = names.length ? `Files opened with the template, for the preview: ${names.join(", ")}.` : "";
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
    ul.append(h("li", { class: "empty" }, "No fields yet. Paste some data above."));
    return;
  }
  const item = (f) => {
    const payload = { type: "field", field: f };
    const btn = h("button", {
      type: "button", class: "field-btn", draggable: "true",
      title: f.type === "List" ? `Drag onto a band: a list that repeats for each item of ${f.name}` : `Drag onto the page, or press Enter to add it to the selected band`,
      "aria-label": `${f.name}, ${f.type}${f.list ? `, in each item of ${f.list}` : ""}. Add to the page`,
      ondragstart: (e) => startDrag(e, payload),
      ondragend: () => { dragPayload = null; clearDrop(); },
      onclick: () => {
        let target = defaultTarget();
        if (f.list) {
          // Put it in the list of its items, if there is one.
          const list = [...index.values()].map((x) => x.node).find((n) => n.tag === "contents" && listFieldOf(n) === f.list);
          if (list) target = list;
        }
        addThing(payload, target, null);
      },
    }, h("span", { class: "name" }, f.name), h("span", { class: "type", dataset: { t: f.type } }, f.type));
    const li = h("li", {}, btn);
    if (f.fromTemplate) li.append(h("p", { class: "note" }, "In the template, not in your data"));
    if (f.fields && f.fields.length) li.append(h("ul", {}, f.fields.map((s) => item({ ...s, list: f.name }))));
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

// ---------------------------------------------------------------------------------------------
// Properties panel

function field(labelText, control, help) {
  const id = control.id || `p-${Math.random().toString(36).slice(2, 8)}`;
  control.id = id;
  return h("div", { class: "prop" }, h("label", { for: id }, labelText), control, help ? h("span", { class: "help" }, help) : null);
}
function seg(labelText, options, value, onPick) {
  const group = h("div", { class: "seg", role: "group", "aria-label": labelText });
  for (const [v, text, aria] of options) {
    group.append(h("button", { type: "button", "aria-pressed": String(v === value), "aria-label": aria || text, title: aria || text, onclick: () => onPick(v) }, text));
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

/** Identifies a control of the panel across a redraw, to keep the keyboard focus on it. */
function focusKey(el) {
  const prop = el.closest(".prop, .grid2, .grid4, .colors");
  const label = prop ? (prop.querySelector(".label, label") || {}).textContent : "";
  return [el.tagName, el.id, el.getAttribute("aria-label"), el.tagName === "BUTTON" ? el.textContent : "", label].join("|");
}

function renderProps() {
  const form = $("props");
  const active = form.contains(document.activeElement) ? focusKey(document.activeElement) : null;
  form.textContent = "";
  if (S.sel && S.sel.type === "el" && index.has(S.sel.id)) elementProps(form, index.get(S.sel.id).node);
  else if (S.sel && S.sel.type === "band" && bandList.some((x) => x.key === S.sel.key)) bandProps(form, bandList.find((x) => x.key === S.sel.key));
  else reportProps(form);
  if (active) {
    const again = [...form.querySelectorAll("button, input, select, textarea")].find((x) => focusKey(x) === active);
    if (again) again.focus({ preventScroll: true });
  }
}

function reportProps(form) {
  const root = S.root;
  const page = pageInfo(root);
  form.append(h("p", { class: "prop-kind" }, "Report and page. Select a part on the page to change it."));
  form.append(field("Report name", h("input", { type: "text", value: root.attrs.name || "", oninput: (e) => change(() => { root.attrs.name = safeName(e.target.value); }, { coalesce: "name", props: false }) })));
  const sizeSel = h("select", { onchange: (e) => change(() => {
    const [w, hh] = PAGE_SIZES[e.target.value];
    setPage(root, { width: page.landscape ? hh : w, height: page.landscape ? w : hh, margins: page.margins });
  }) }, [...Object.keys(PAGE_SIZES), ...(page.size === "Custom" ? ["Custom"] : [])].map((k) => h("option", { value: k, selected: k === page.size }, k === "Custom" ? `Custom (${page.width} × ${page.height})` : k)));
  form.append(field("Page size", sizeSel));
  form.append(seg("Orientation", [["portrait", "Portrait"], ["landscape", "Landscape"]], page.landscape ? "landscape" : "portrait", (v) => change(() => {
    if ((v === "landscape") === page.landscape) return false;
    setPage(root, { width: page.height, height: page.width, margins: page.margins });
  })));
  const margins = h("div", { class: "grid4" });
  for (const k of ["top", "bottom", "left", "right"]) {
    margins.append(h("label", {}, k[0].toUpperCase() + k.slice(1), numberInput(page.margins[k], (v) => change(() => {
      setPage(root, { width: page.width, height: page.height, margins: { ...pageInfo(root).margins, [k]: Math.max(0, v) } });
    }, { coalesce: `margin:${k}`, props: false }), { min: "0" })));
  }
  form.append(h("div", { class: "prop" }, h("span", { class: "label" }, "Margins (points)"), margins));
  form.append(field("Language of the preview", h("select", { onchange: (e) => { S.locale = e.target.value; schedulePreview(); scheduleSave(); } },
    Object.entries(LOCALES).map(([k, v]) => h("option", { value: k, selected: k === S.locale }, v))), "How numbers and dates are written: 1,234.50 or 1.234,50."));
  form.append(h("div", { class: "prop" }, h("label", { class: "inline" }, h("input", { type: "checkbox", id: "auto-heading", checked: localStorage.getItem("ferroreport.designer.headings") !== "no", onchange: (e) => localStorage.setItem("ferroreport.designer.headings", e.target.checked ? "yes" : "no") }), "Fields dropped on Detail get a heading in the column header")));
  const grid = h("select", { onchange: (e) => { S.grid = Number(e.target.value); renderCanvas(); scheduleSave(); } }, [2, 5, 10].map((g) => h("option", { value: String(g), selected: g === S.grid }, `${g} points`)));
  form.append(field("Grid", grid));
  form.append(h("p", { class: "help" }, "1 point is 1/72 of an inch. A4 is 595 × 842 points."));
}

function bandProps(form, b) {
  form.append(h("p", { class: "prop-kind" }, `${b.label} band. ${b.help}.`));
  const hh = b.node ? num(b.node, "height") : 0;
  const minH = b.node ? elementsOf(b.node).reduce((m, el) => Math.max(m, num(el, "y") + num(el, "height")), 0) : 0;
  form.append(field("Height (points)", numberInput(hh, (v) => change(() => {
    const node = b.node || b.create();
    node.attrs.height = String(Math.max(minH, Math.round(v)));
  }, { coalesce: `band:${b.key}`, props: false }), { min: String(minH) }), hh === 0 ? "This band is empty, so it takes no room. Give it a height or drop something on it." : "Drag the bottom edge of the band on the page to change it too."));
  if (b.node && elementsOf(b.node).length) {
    form.append(h("div", { class: "prop-actions" }, h("button", { type: "button", class: "danger", onclick: () => {
      change(() => { b.node.children = b.node.children.filter((c) => c.tag !== "element"); b.node.attrs.height = "0"; });
      announce(`${b.label} emptied. Ctrl+Z brings it back.`);
    } }, "Empty this band")));
  }
}

function elementProps(form, el) {
  const kind = el.attrs.kind;
  const { parent } = index.get(el._id);
  const ed = (fn, key) => change(fn, { coalesce: key ? `${el._id}:${key}` : null, props: !key });
  form.append(h("p", { class: "prop-kind" }, elementLabel(el)));

  if (kind === "staticText") {
    form.append(field("Text", h("textarea", { id: "prop-text", rows: "3", oninput: (e) => ed(() => setMainText(el, e.target.value), "text") }, (child(el, "text") || {}).text || "")));
  }
  if (kind === "textField") {
    const expr = (child(el, "expression") || {}).text || "";
    const ta = h("textarea", { id: "prop-expr", rows: "2", spellcheck: "false", oninput: (e) => ed(() => setMainText(el, e.target.value), "expr") }, expr);
    const names = [...new Set([...declaredFields(datasetOfContainer(S.root, parent, index) || S.root).map((f) => f.name)])];
    const insert = h("select", { "aria-label": "Field to put in", onchange: (e) => {
      if (!e.target.value) return;
      const v = e.target.value;
      ed(() => setMainText(el, v));
    } }, h("option", { value: "" }, "Show a field…"), names.map((n) => h("option", { value: `$F{${n}}` }, n)),
      h("option", { value: "$V{PAGE_NUMBER}" }, "Page number"), h("option", { value: "new java.util.Date()" }, "Today's date"));
    form.append(h("div", { class: "prop" }, h("label", { for: "prop-expr" }, "Shows"), ta, h("div", { class: "insert-row" }, insert),
      h("span", { class: "help" }, "A field is written $F{name}. Join text with +, as in \"Total: \" + $F{amount}.")));
    const pat = el.attrs.pattern || "";
    const known = PATTERNS.some(([p]) => p === pat);
    const patSel = h("select", { onchange: (e) => ed(() => setAttr(el, "pattern", e.target.value === "custom" ? pat || "0.00" : e.target.value)) },
      PATTERNS.map(([p, ex]) => h("option", { value: p, selected: p === pat }, p ? `${ex}  (${p})` : ex)), h("option", { value: "custom", selected: !known }, "Custom…"));
    form.append(field("Number or date format", patSel));
    if (!known) form.append(field("Custom format", h("input", { type: "text", value: pat, oninput: (e) => ed(() => setAttr(el, "pattern", e.target.value), "pattern") }), "As in Java: #,##0.00 for numbers, dd/MM/yyyy for dates."));
    form.append(h("div", { class: "prop toggles" },
      h("label", { class: "inline" }, h("input", { type: "checkbox", checked: el.attrs.textAdjust === "StretchHeight", onchange: (e) => ed(() => setAttr(el, "textAdjust", e.target.checked ? "StretchHeight" : null)) }), "Grow to fit long text"),
      h("label", { class: "inline" }, h("input", { type: "checkbox", checked: el.attrs.blankWhenNull === "true", onchange: (e) => ed(() => setAttr(el, "blankWhenNull", e.target.checked ? "true" : null)) }), "Blank when there is no value")));
  }

  // Position and size
  const g = geo(el);
  const pos = h("div", { class: "grid4" });
  for (const [k, label] of [["x", "X"], ["y", "Y"], ["width", "Width"], ["height", "Height"]]) {
    pos.append(h("label", {}, label, numberInput(g[{ x: "x", y: "y", width: "w", height: "h" }[k]], (v) => ed(() => {
      setGeo(el, { [{ x: "x", y: "y", width: "w", height: "h" }[k]]: v });
      fit(el, parent);
    }, `geo:${k}`), { min: "0", "aria-label": `${label} in points` })));
  }
  form.append(h("div", { class: "prop" }, h("span", { class: "label" }, "Position and size (points)"), pos));

  const st = effectiveStyle(S.root, el);
  if (kind === "staticText" || kind === "textField") {
    const size = parseFloat(st.fontSize) || 10;
    form.append(h("div", { class: "grid2" },
      h("label", {}, "Font size", numberInput(size, (v) => ed(() => setAttr(el, "fontSize", Math.max(1, v)), "fontSize"), { min: "1", step: "0.5" })),
      h("label", {}, "Font", h("select", { onchange: (e) => ed(() => setAttr(el, "fontName", e.target.value)) },
        ["DejaVu Sans", "DejaVu Serif", "DejaVu Sans Mono"].map((f) => h("option", { value: f, selected: (st.fontName || "DejaVu Sans") === f }, f.replace("DejaVu ", "").replace("Sans Mono", "Monospace")))))));
    const toggle = (attr, text, aria) => h("button", { type: "button", "aria-pressed": String(st[attr] === "true"), "aria-label": aria, title: aria, style: { fontWeight: attr === "bold" ? "800" : "650", fontStyle: attr === "italic" ? "italic" : "normal", textDecoration: attr === "underline" ? "underline" : "none" },
      onclick: () => ed(() => setAttr(el, attr, st[attr] === "true" ? (el.attrs.style || kids(S.root, "style").some((s) => s.attrs.default === "true" && s.attrs[attr] === "true") ? "false" : null) : "true")) }, text);
    form.append(h("div", { class: "prop" }, h("span", { class: "label" }, "Style"), h("div", { class: "seg", role: "group", "aria-label": "Style" }, toggle("bold", "B", "Bold"), toggle("italic", "I", "Italic"), toggle("underline", "U", "Underline"))));
    form.append(seg("Align", [["Left", "Left"], ["Center", "Center"], ["Right", "Right"], ["Justified", "Justify"]], st.hTextAlign || "Left", (v) => ed(() => setAttr(el, "hTextAlign", v === "Left" ? null : v))));
    form.append(seg("Vertical", [["Top", "Top"], ["Middle", "Middle"], ["Bottom", "Bottom"]], st.vTextAlign || "Top", (v) => ed(() => setAttr(el, "vTextAlign", v === "Top" ? null : v))));
    const filled = st.mode === "Opaque";
    form.append(h("div", { class: "prop" }, h("span", { class: "label" }, "Colors"), h("div", { class: "colors" },
      h("label", {}, "Text", colorInput(st.forecolor, (v) => ed(() => setAttr(el, "forecolor", v), "fore"), "Text color")),
      h("label", {}, "Background", colorInput(st.backcolor || "#FFFFFF", (v) => ed(() => { setAttr(el, "backcolor", v); el.attrs.mode = "Opaque"; }, "back"), "Background color")),
      h("label", { class: "inline" }, h("input", { type: "checkbox", checked: !filled, onchange: (e) => ed(() => { if (e.target.checked) { el.attrs.mode = "Transparent"; } else { el.attrs.mode = "Opaque"; if (!st.backcolor) el.attrs.backcolor = "#FFFFFF"; } }) }), "No background"))));
    bordersProps(form, el, ed);
  }
  if (kind === "line" || kind === "rectangle" || kind === "ellipse") {
    const pen = readPen(el);
    const colors = h("div", { class: "colors" }, h("label", {}, kind === "line" ? "Color" : "Border", colorInput(st.forecolor, (v) => ed(() => setAttr(el, "forecolor", v), "fore"), kind === "line" ? "Line color" : "Border color")));
    if (kind !== "line") {
      const filled = st.mode !== "Transparent";
      colors.append(h("label", {}, "Fill", colorInput(st.backcolor || "#FFFFFF", (v) => ed(() => { setAttr(el, "backcolor", v); delete el.attrs.mode; }, "back"), "Fill color")));
      colors.append(h("label", { class: "inline" }, h("input", { type: "checkbox", checked: !filled, onchange: (e) => ed(() => setAttr(el, "mode", e.target.checked ? "Transparent" : null)) }), "No fill"));
    }
    form.append(h("div", { class: "prop" }, h("span", { class: "label" }, "Colors"), colors));
    form.append(h("div", { class: "grid2" },
      h("label", {}, "Thickness", numberInput(pen.width, (v) => ed(() => writePen(el, { ...readPen(el), width: Math.max(0, v) }), "pen"), { min: "0", step: "0.25" })),
      h("label", {}, "Line", h("select", { onchange: (e) => ed(() => writePen(el, { ...readPen(el), style: e.target.value })) },
        ["Solid", "Dashed", "Dotted", "Double"].map((s) => h("option", { value: s, selected: s === pen.style }, s))))));
    if (kind === "rectangle") form.append(field("Rounded corners (points)", numberInput(num(el, "radius"), (v) => ed(() => setAttr(el, "radius", v > 0 ? Math.round(v) : null), "radius"), { min: "0" })));
    if (kind === "line" && g.w > 1 && g.h > 1) form.append(seg("Direction", [["TopDown", "↘", "Top left to bottom right"], ["BottomUp", "↗", "Bottom left to top right"]], el.attrs.direction || "TopDown", (v) => ed(() => setAttr(el, "direction", v === "TopDown" ? null : v))));
  }
  if (kind === "image") {
    const file = h("input", { type: "file", id: "prop-image-file", accept: "image/png,image/jpeg,image/svg+xml", hidden: true, onchange: async (e) => {
      const f = e.target.files[0];
      if (!f) return;
      if (f.size > 2 * 1024 * 1024) { announce("That image is over 2 MB: please use a smaller one.", "bad"); return; }
      const url = await new Promise((res) => { const r = new FileReader(); r.onload = () => res(r.result); r.readAsDataURL(f); });
      change(() => { setMainText(el, `"${url}"`); });
      announce(`Image ${f.name} added`);
    } });
    form.append(h("div", { class: "prop" }, h("span", { class: "label" }, "Picture"), h("div", { class: "row-btns" }, h("button", { type: "button", onclick: () => file.click() }, "Choose an image…"), file),
      h("span", { class: "help" }, "PNG, JPEG or SVG. It is kept inside the template.")));
    form.append(field("Fit", h("select", { onchange: (e) => ed(() => setAttr(el, "scaleImage", e.target.value)) },
      [["RetainShape", "Fit inside, keep its shape"], ["FillFrame", "Stretch to fill"], ["Clip", "Real size, cropped"]].map(([v, t]) => h("option", { value: v, selected: (el.attrs.scaleImage || "RetainShape") === v }, t)))));
  }
  if (kind === "frame") {
    const filled = st.mode === "Opaque";
    form.append(h("div", { class: "prop" }, h("span", { class: "label" }, "Background"), h("div", { class: "colors" },
      colorInput(st.backcolor || "#FFFFFF", (v) => ed(() => { setAttr(el, "backcolor", v); el.attrs.mode = "Opaque"; }, "back"), "Background color"),
      h("label", { class: "inline" }, h("input", { type: "checkbox", checked: !filled, onchange: (e) => ed(() => setAttr(el, "mode", e.target.checked ? null : "Opaque")) }), "None"))));
    bordersProps(form, el, ed);
  }
  if (!["staticText", "textField", "line", "rectangle", "ellipse", "image", "frame"].includes(kind)) {
    const inner = innerContainer(el);
    form.append(h("p", { class: "kept" }, inner
      ? `This list repeats what is inside it once for each item of “${listFieldOf(inner) || "its data"}”. Select the parts inside it to change them, or drop fields of those items into it.`
      : "The designer cannot change this kind of part yet. It stays in the template as it is, and the preview shows it."));
  }

  // When it prints
  const pw = child(el, "printWhenExpression");
  form.append(field("Show only when (optional)", h("input", { type: "text", value: pw ? pw.text || "" : "", placeholder: "e.g. $F{overdue}", spellcheck: "false", oninput: (e) => ed(() => {
    let n = child(el, "printWhenExpression");
    if (!e.target.value.trim()) { if (n) el.children.splice(el.children.indexOf(n), 1); return; }
    if (!n) { n = make("printWhenExpression", {}, [], ""); el.children.unshift(n); }
    n.text = e.target.value;
  }, "printWhen") }), "Leave empty to always show it."));

  form.append(h("div", { class: "prop-actions" },
    h("button", { type: "button", onclick: duplicateSelected }, "Duplicate"),
    h("button", { type: "button", onclick: () => reorder(true) }, "To front"),
    h("button", { type: "button", onclick: () => reorder(false) }, "To back"),
    h("button", { type: "button", class: "danger", onclick: removeSelected }, "Delete")));
}

function bordersProps(form, el, ed) {
  const b = readBorders(el);
  const any = Object.values(b.sides).some(Boolean);
  const write = (patch) => ed(() => {
    const cur = readBorders(el);
    const next = { ...cur, ...patch, sides: { ...cur.sides, ...(patch.sides || {}) } };
    if (!Object.values(cur.sides).some(Boolean) && !patch.sides && patch.width) next.sides = { top: true, left: true, bottom: true, right: true };
    if (next.width === 0) next.width = 0.5;
    writeBorders(el, next);
  }, patchKey(patch));
  const patchKey = (p) => (p.color ? "borderColor" : null);
  const sides = h("div", { class: "seg", role: "group", "aria-label": "Sides with a border" },
    [["top", "Top"], ["bottom", "Bottom"], ["left", "Left"], ["right", "Right"]].map(([s, t]) => h("button", { type: "button", "aria-pressed": String(b.sides[s]), "aria-label": `${t} border`, title: `${t} border`, onclick: () => write({ sides: { [s]: !b.sides[s] } }) }, t)));
  form.append(h("div", { class: "prop" }, h("span", { class: "label" }, "Borders"), sides,
    h("div", { class: "colors" },
      h("label", {}, "Width", h("select", { onchange: (e) => { const w = Number(e.target.value); if (w === 0) ed(() => writeBorders(el, { ...readBorders(el), width: 0 })); else write({ width: w }); } },
        [[0, "None"], [0.5, "Thin"], [1, "1 pt"], [2, "2 pt"], [3, "3 pt"]].map(([v, t]) => h("option", { value: String(v), selected: any ? Math.abs(b.width - v) < 0.01 : v === 0 }, t)))),
      h("label", {}, "Color", colorInput(b.color, (v) => write({ color: v }), "Border color")),
      h("label", {}, "Padding", numberInput(num(child(el, "box") || { attrs: {} }, "padding"), (v) => ed(() => {
        let box = child(el, "box");
        if (!box) { box = make("box"); el.children.push(box); }
        setAttr(box, "padding", v > 0 ? Math.round(v) : null);
        if (!box.children.length && !Object.keys(box.attrs).length) el.children.splice(el.children.indexOf(box), 1);
      }, "padding"), { min: "0", "aria-label": "Padding in points" })))));
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

function renderPreview() {
  if (!engine || !S.preview) return;
  const msg = $("preview-msg");
  const errBox = $("preview-error");
  try {
    const t0 = performance.now();
    const pdf = renderTo("pdf");
    const ms = performance.now() - t0;
    const url = URL.createObjectURL(new Blob([pdf], { type: "application/pdf" }));
    const frame = $("preview");
    const old = previewUrl;
    previewUrl = url;
    frame.src = `${url}#toolbar=0&navpanes=0&view=FitH`;
    if (old) setTimeout(() => URL.revokeObjectURL(old), 4000);
    errBox.hidden = true;
    msg.textContent = `Up to date · made in ${ms < 10 ? ms.toFixed(1) : Math.round(ms)} ms · ${(pdf.length / 1024).toFixed(0)} KB`;
    msg.className = "msg good";
    document.body.dataset.previewBytes = String(pdf.length);
    document.body.dataset.previewCount = String(Number(document.body.dataset.previewCount || 0) + 1);
  } catch (e) {
    msg.textContent = "Not updated: the report has a problem";
    msg.className = "msg bad";
    errBox.hidden = false;
    const text = String(e.message || e);
    const missing = /`([^`]+)` not found in the resource directories/.exec(text);
    errBox.textContent = missing
      ? `This report uses the file ${missing[1]}. Open it together with the template: in Open…, select both.`
      : /null used as a boolean|on null/.test(text) && !S.rows.length
        ? `${text}\n\nThe report needs data: paste a sample on the left.`
        : text;
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
    announce(`Saved ${baseName()}.${to}`);
  } catch (e) {
    announce(`Could not save: ${e.message || e}`, "bad");
  }
}

function exportReport(format) {
  closeMenus();
  const types = { pdf: "application/pdf", xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document" };
  try {
    download(renderTo(format), `${baseName()}.${format}`, types[format]);
    announce(`Downloaded ${baseName()}.${format}`);
  } catch (e) {
    announce(`Could not make the ${format.toUpperCase()}: ${e.message || e}`, "bad");
  }
}

// ---------------------------------------------------------------------------------------------
// Opening, starting and keeping work

/** Reads a template (and the files it imports) into the designer. */
function openTemplate(text, resources = {}, name = "report") {
  const tree = engine.toTree(text, { resources });
  S.resources = resources;
  if (tree.tag !== "jasperReport") throw new Error("This file is a style template, not a report.");
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
      S.root = blankReport("Report");
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
    fitZoom();
    refresh();
    announce(kind === "blank" ? "A blank report. Drag fields from the left onto the page." : `Opened the ${kind === "invoice" ? "invoices" : "employees"} example`);
  } catch (e) {
    announce(`Could not open it: ${e.message || e}`, "bad");
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
    fitZoom();
    refresh();
    announce(main ? `Opened ${main.name}${data ? ` with the data in ${data.name}` : madeUp ? ", with made-up data for a preview: paste yours on the left" : ""}` : `Data from ${data.name}`);
  } catch (e) {
    const msg = String(e.message || e);
    announce(/import/.test(msg) ? `${main.name} uses other files (import): open it together with them (select them all in Open…).` : `Could not open ${main ? main.name : "the file"}: ${msg}`, "bad");
  }
}

let saveTimer = null;
function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    try {
      localStorage.setItem(STORE, JSON.stringify({ tree: toWire(S.root), data: S.dataText, params: S.paramsText, name: S.name, locale: S.locale, grid: S.grid, snap: S.snap, zoom: S.zoom }));
    } catch { /* full or private mode: the work stays on the page */ }
  }, 300);
}

function closeMenus() {
  document.querySelectorAll("details.menu[open]").forEach((d) => d.removeAttribute("open"));
}

function fitZoom() {
  const stage = $("canvas-stage");
  const avail = stage.clientWidth - GUTTER - 28;
  const w = pageInfo(S.root).width * PT;
  S.zoom = Math.max(0.25, Math.min(1.5, Math.floor((avail / w) * 100) / 100));
}
function setZoom(z) {
  S.zoom = Math.max(0.25, Math.min(4, Math.round(z * 100) / 100));
  renderCanvas();
  scheduleSave();
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
  // Double-click a text to edit what it says.
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
    if (f) { setData(await f.text()); announce(`Data from ${f.name}`); }
    e.target.value = "";
  });

  document.querySelectorAll("[data-start]").forEach((b) => b.addEventListener("click", () => startWith(b.dataset.start)));
  $("open-btn").addEventListener("click", () => { closeMenus(); $("open-file").click(); });
  $("open-file").addEventListener("change", (e) => { if (e.target.files.length) openFiles(e.target.files); e.target.value = ""; });
  $("save-ferro").addEventListener("click", () => saveTemplate("ferro"));
  $("save-jrxml").addEventListener("click", () => saveTemplate("jrxml"));
  document.querySelectorAll("[data-export]").forEach((b) => b.addEventListener("click", () => exportReport(b.dataset.export)));
  document.addEventListener("click", (e) => { document.querySelectorAll("details.menu[open]").forEach((d) => { if (!d.contains(e.target)) d.removeAttribute("open"); }); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") document.querySelectorAll("details.menu[open]").forEach((d) => { d.removeAttribute("open"); d.querySelector("summary").focus(); }); });
  document.querySelectorAll("details.menu").forEach((d) => d.addEventListener("toggle", () => {
    if (d.open) document.querySelectorAll("details.menu[open]").forEach((o) => { if (o !== d) o.removeAttribute("open"); });
  }));

  $("undo").addEventListener("click", undo);
  $("redo").addEventListener("click", redo);
  $("zoom-in").addEventListener("click", () => setZoom(S.zoom * 1.25));
  $("zoom-out").addEventListener("click", () => setZoom(S.zoom / 1.25));
  $("zoom-label").addEventListener("click", () => { fitZoom(); renderCanvas(); });
  $("snap").addEventListener("change", (e) => { S.snap = e.target.checked; renderCanvas(); scheduleSave(); });
  $("toggle-preview").addEventListener("click", (e) => {
    S.preview = !S.preview;
    e.currentTarget.setAttribute("aria-pressed", String(S.preview));
    document.querySelector(".center").classList.toggle("no-preview", !S.preview);
    if (S.preview) renderPreview();
  });
  $("help-btn").addEventListener("click", () => $("help").showModal());
}

async function boot() {
  wire();
  try {
    engine = await import("../pkg/ferroreport.js");
    await engine.default();
  } catch (e) {
    $("preview-msg").textContent = "The engine could not be loaded";
    $("preview-msg").className = "msg bad";
    $("preview-error").hidden = false;
    $("preview-error").textContent = `The WebAssembly engine did not load (${e.message || e}). Try a current Chrome, Firefox, Safari or Edge.`;
    return;
  }
  const params = new URLSearchParams(location.search);
  const start = params.get("start");
  let saved = null;
  try { saved = JSON.parse(localStorage.getItem(STORE) || "null"); } catch { saved = null; }
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
      if (saved.zoom) S.zoom = saved.zoom; else fitZoom();
      refresh();
      announce("Your last report, as you left it");
      return;
    } catch { /* fall through to the example */ }
  }
  await startWith(["blank", "invoice", "employees"].includes(start) ? start : "invoice");
}

// For the browser console and automated tests: the template being designed.
window.designer = {
  tree: () => toWire(S.root),
  template: (to = "jrxml") => templateText(to),
  rows: () => S.rows,
};

boot();
