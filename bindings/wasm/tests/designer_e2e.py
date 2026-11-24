"""The report designer in a real browser, as a person uses it.

    bindings/wasm/build.sh && tools/assemble-site.sh _site
    pip install playwright && python -m playwright install chromium
    python3 bindings/wasm/tests/designer_e2e.py _site [--shots DIR] [--headed] [--chrome PATH]

Serves the assembled site on a free local port and checks, in Chromium: the first-run guide
(once), the invoice example, dragging a field onto the page, text with {field} in the Text box
(typed, and chosen from the list that { opens), a static text that becomes a field, undo and
redo, the keyboard, saving .ferro and .jrxml and opening them again (the same JRXML comes back),
downloading PDF, Excel and Word, a template with formulas (shown under Advanced, saved as they
were), a blank report, and the Spanish interface. With --shots, saves screenshots there."""
import argparse
import functools
import http.server
import json
import os
import re
import tempfile
import threading
import time
import xml.etree.ElementTree as ET

from playwright.sync_api import sync_playwright

ap = argparse.ArgumentParser()
ap.add_argument("site")
ap.add_argument("--shots")
ap.add_argument("--headed", action="store_true")
ap.add_argument("--chrome", help="a Chrome or Chromium to use instead of Playwright's")
args = ap.parse_args()
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../.."))
OUT = tempfile.mkdtemp(prefix="designer-e2e-")
started = time.time()


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=os.path.abspath(args.site)))
threading.Thread(target=server.serve_forever, daemon=True).start()
BASE = f"http://127.0.0.1:{server.server_address[1]}/designer/"


def ok(cond, msg):
    if not cond:
        raise AssertionError(msg)
    print("ok -", msg)


def shot(page, name):
    if args.shots:
        os.makedirs(args.shots, exist_ok=True)
        page.screenshot(path=os.path.join(args.shots, f"{name}.png"))


def canonical(xml):
    """A JRXML as a tree that does not depend on the order of attributes or on indentation."""
    def walk(e):
        return (e.tag, tuple(sorted(e.attrib.items())), (e.text or "").strip(), tuple(walk(c) for c in e))
    return walk(ET.fromstring(xml.encode("utf-8")))


def expressions(xml):
    """The expressions of a JRXML (6 or 7), stripped."""
    return [m.strip() for m in re.findall(r"<(?:\w*[eE]xpression)[^>]*><!\[CDATA\[(.*?)\]\]>", xml, re.S)]


with sync_playwright() as p:
    launch = {"headless": not args.headed}
    if args.chrome:
        launch["executable_path"] = args.chrome
    browser = p.chromium.launch(**launch)
    errors = []

    def new_page(width=1920, height=1080, welcomed=True, lang="en"):
        ctx = browser.new_context(viewport={"width": width, "height": height}, locale="en-US", accept_downloads=True)
        if welcomed:
            ctx.add_init_script("try { localStorage.setItem('ferroreport.designer.welcomed', 'yes') } catch (e) {}")
        if lang:
            ctx.add_init_script(f"try {{ if (!sessionStorage.getItem('langSet')) {{ localStorage.setItem('ferroreport.designer.lang', '{lang}'); sessionStorage.setItem('langSet', '1') }} }} catch (e) {{}}")
        pg = ctx.new_page()
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.on("console", lambda m: errors.append(m.text) if m.type == "error" and "favicon" not in m.text and "404" not in m.text else None)
        return pg

    def helpers(pg):
        count = lambda: int(pg.evaluate("document.body.dataset.previewCount || 0"))
        def wait_preview(after):
            pg.wait_for_function(f"Number(document.body.dataset.previewCount || 0) > {after}", timeout=20000)
        return count, wait_preview

    def download(pg, click):
        with pg.expect_download() as d:
            click()
        path = os.path.join(OUT, d.value.suggested_filename)
        d.value.save_as(path)
        return path

    # 1. The first visit shows the guide once; it is gone after it is closed.
    pg = new_page(welcomed=False)
    count, wait_preview = helpers(pg)
    pg.goto(BASE)
    wait_preview(0)
    guide = pg.locator("#guide")
    guide.wait_for(state="visible")
    ok("three steps" in guide.inner_text() and pg.get_by_role("button", name="Start from the invoice example").is_visible(), "the first visit shows the three-step guide")
    shot(pg, "guide")
    pg.get_by_role("button", name="Close").first.click()
    ok(not guide.is_visible(), "the guide closes")
    pg.reload()
    wait_preview(0)
    pg.wait_for_timeout(300)
    ok(not guide.is_visible(), "and it does not come back on the next visit")
    pg.locator("#help-menu > summary").click()
    pg.get_by_role("button", name="Getting started").click()
    ok(guide.is_visible(), "Help, Getting started shows it again")
    pg.get_by_role("button", name="Start blank").click()
    pg.wait_for_timeout(300)
    ok(not guide.is_visible() and "Ana Peña" in pg.input_value("#data"), "Start blank in the guide starts a blank report")
    pg.context.close()

    # 2. The invoice example: bands with plain names, fields as pills, no code on the page.
    pg = new_page()
    count, wait_preview = helpers(pg)
    tpl = lambda to="jrxml": pg.evaluate(f"designer.template('{to}')")
    pg.goto(BASE + "?start=invoice")
    wait_preview(0)
    pg.wait_for_timeout(500)
    shot(pg, "en-1920")
    labels = pg.locator(".band-label b, .band-strip b").all_inner_texts()
    ok(labels == ["Report title", "Top of every page", "Column titles", "Each row of your data", "Below the rows", "Bottom of every page", "End of the report"], f"bands have plain names: {labels}")
    ok(pg.locator(".band-strip").count() == 5 and "Add something here" in pg.locator(".band-strip").first.inner_text(), "empty bands are slim strips that invite a drop")
    ok(pg.locator(".band-strip").first.bounding_box()["height"] <= 26, "and they are slim")
    pills = pg.locator('[data-band-key="detail:0"] .pill').all_inner_texts()
    ok("number" in pills and "customer" in pills, f"fields show as pills: {pills}")
    ok("$F{" not in pg.inner_text("#sheet"), "no $F{} on the page")

    # Select the invoice line: its Text box reads {number} · {customer}.
    pg.locator('[data-band-key="detail:0"] .el', has=pg.locator(".pill", has_text="number")).first.click()
    ok(pg.input_value("#prop-text") == "{number} · {customer}", "the Text box shows {number} · {customer}")
    ok(pg.locator(".psec h3").all_inner_texts()[:3] == ["Text", "Look", "Alignment"], "the panel is in sections: Text, Look, Alignment...")
    shot(pg, "text-editor")

    # 3. Drag a field onto the empty "Bottom of every page" strip.
    c0, b0 = count(), pg.evaluate("document.body.dataset.previewBytes")
    footer = pg.locator('.band-strip[data-band-key="pageFooter"]')
    pg.locator(".field-btn", has_text="customer").first.drag_to(footer, target_position={"x": 300, "y": 8})
    wait_preview(c0)
    sel = pg.locator(".el.selected")
    ok(sel.count() == 1 and sel.locator(".pill").inner_text() == "customer", "the dropped field is selected and shows a pill")
    ok(pg.input_value("#prop-text") == "{customer}", "its Text box shows {customer}")
    ok(pg.evaluate("document.body.dataset.previewBytes") != b0, "the preview PDF changed")
    jr = tpl()
    ok(re.search(r"<pageFooter height=\"\d+\">\s*<element kind=\"textField\"[^>]*>\s*<expression><!\[CDATA\[\$F\{customer\}\]\]>", jr), "the page footer has the field in the template")

    # 4. Type text with a field; then choose a field from the list that { opens.
    c0 = count()
    pg.locator("#prop-text").fill("Customer: {customer}")
    wait_preview(c0)
    ok('"Customer: " + $F{customer}' in tpl(), "typed text becomes \"Customer: \" + $F{customer}")
    ok(sel.inner_text().strip() == "Customer: customer" and sel.locator(".pill").inner_text() == "customer", "the canvas shows the words and the pill")
    box = pg.locator("#prop-text")
    box.fill("")
    box.type("Invoice ")
    box.type("{")
    ok(pg.locator("#picker").is_visible(), "typing { opens the list of fields")
    box.type("num")
    ok(pg.locator("#picker li").all_inner_texts()[0].startswith("number"), "typing narrows it")
    box.press("Enter")
    ok(box.input_value() == "Invoice {number}", "Enter puts {number} in the text: " + box.input_value())
    pg.wait_for_timeout(600)
    ok('"Invoice " + $F{number}' in tpl(), "and the template has \"Invoice \" + $F{number}")
    pg.locator(".insert-btn").click()
    pg.locator("#picker li", has_text="page number").click()
    ok(box.input_value() == "Invoice {number}$V{PAGE_NUMBER}", "Insert a field adds a value where the caret is")
    box.fill("Invoice {number}")

    # A static text that gets a field becomes a field.
    pg.locator('[data-select-band="title"]').click()
    pg.locator('#parts button[data-part="staticText"]').click()
    ok(pg.evaluate("document.activeElement.id") == "prop-text", "a new Text part puts the caret in its Text box")
    pg.keyboard.type("Hello {customer}")
    pg.wait_for_timeout(600)
    jr = tpl()
    ok('"Hello " + $F{customer}' in jr, "a Text part with {customer} becomes a text field")
    ok(pg.locator("#props-h").inner_text() == "Text with fields", "and the panel says so")

    # 5. Undo and redo, with the buttons and the keyboard.
    pg.get_by_role("button", name="Undo (Ctrl+Z)").click()
    ok('"Hello " + $F{customer}' not in tpl(), "Undo takes the last change back")
    pg.get_by_role("button", name="Redo (Ctrl+Y)").click()
    ok('"Hello " + $F{customer}' in tpl(), "Redo does it again")
    el = pg.locator('[data-band-key="title"] .el', has=pg.locator(".pill", has_text="customer")).first
    el.focus()
    def title_el():
        t = pg.evaluate("designer.tree()")
        title = next(c for c in t["children"] if c["tag"] == "title")
        for e in title["children"]:
            if any("Hello" in (k.get("text") or "") for k in e["children"]):
                return dict(e["attrs"])
    a0 = title_el()
    pg.keyboard.press("ArrowRight")
    pg.keyboard.press("ArrowRight")
    pg.keyboard.press("Shift+ArrowDown")
    a1 = title_el()
    ok(int(a1["x"]) == int(a0["x"]) + 10 and int(a1["height"]) == int(a0["height"]) + 5, f"arrow keys move and resize ({a0['x']},{a0['height']} -> {a1['x']},{a1['height']})")
    pg.keyboard.press("Delete")
    ok(title_el() is None, "Delete removes it")
    pg.keyboard.press("Control+z")
    ok(title_el() is not None, "Ctrl+Z brings it back")
    pg.keyboard.press("Control+y")
    ok(title_el() is None, "Ctrl+Y removes it again")
    pg.keyboard.press("Control+z")

    # 6. Save .ferro and .jrxml; open each again: the same template comes back.
    jr_now = tpl()
    pg.get_by_text("File", exact=True).click()
    ferro_path = download(pg, lambda: pg.get_by_role("button", name="Save template (.ferro)").click())
    ferro = open(ferro_path, encoding="utf-8").read()
    ok(ferro.startswith("report Invoices") and '"Invoice " + $F{number}' in ferro and '"Hello " + $F{customer}' in ferro, f"saved {os.path.basename(ferro_path)}")
    pg.get_by_text("File", exact=True).click()
    jrxml_path = download(pg, lambda: pg.get_by_role("button", name="Save for JasperReports (.jrxml)").click())
    saved_jr = open(jrxml_path, encoding="utf-8").read()
    ok(saved_jr == jr_now and "<![CDATA[\"Invoice \" + $F{number}]]>" in saved_jr, f"saved {os.path.basename(jrxml_path)}")
    for path in (ferro_path, jrxml_path):
        pg.get_by_text("File", exact=True).click()
        pg.get_by_role("button", name="Blank report").click()
        pg.wait_for_timeout(200)
        c0 = count()
        pg.locator("#open-file").set_input_files(path)
        wait_preview(c0)
        ok(canonical(tpl()) == canonical(jr_now), f"opening the saved {os.path.splitext(path)[1]} gives the same JRXML")
    ok(pg.locator('[data-band-key="pageFooter"] .pill', has_text="number").count() == 1, "and shows it on the page")
    pg.locator("#open-file").set_input_files(os.path.join(ROOT, "examples/invoices.json"))
    pg.wait_for_timeout(800)
    ok(pg.inner_text("#data-msg").startswith("3 rows"), "data opened from a JSON file")

    # 7. Downloads: PDF from the main button, Excel and Word from its menu.
    pdf = download(pg, lambda: pg.get_by_role("button", name="Download PDF").click())
    ok(open(pdf, "rb").read(5) == b"%PDF-", f"Download PDF gives {os.path.basename(pdf)}")
    for fmt, label in (("xlsx", "Excel (.xlsx)"), ("docx", "Word (.docx)")):
        pg.locator("#download-menu > summary").click()
        path = download(pg, lambda: pg.get_by_role("button", name=label).click())
        ok(open(path, "rb").read(2) == b"PK" and path.endswith(fmt), f"downloads {os.path.basename(path)}")

    # The preview tab shows the PDF.
    pg.get_by_role("tab", name="Preview").click()
    ok(pg.locator("#preview").is_visible() and not pg.locator("#canvas-stage").is_visible(), "the Preview tab shows the PDF in place of the page")
    ok("up to date" in pg.inner_text("#preview-msg"), "and says it is up to date")
    pg.get_by_role("tab", name="Design").click()

    # 8. Formulas: shown under Advanced, saved as they were.
    fx = os.path.join(ROOT, "fixtures/06-expressions")
    original = open(os.path.join(fx, "report.jrxml"), encoding="utf-8").read()
    c0 = count()
    pg.locator("#open-file").set_input_files([os.path.join(fx, "report.jrxml"), os.path.join(fx, "data.json")])
    wait_preview(c0)
    pg.wait_for_timeout(300)
    formula = "$F{label}.length() > 12 ? $F{label}.substring(0, 12) + \"~\" : $F{label}.toLowerCase()"
    pg.locator(".el", has=pg.locator(".code", has_text="length()")).first.click()
    ok(pg.locator("#prop-text").count() == 0 and "worked out with a formula" in pg.inner_text(".formula-note"), "a formula is not put in the Text box")
    ok(pg.locator('details[data-sec="adv"]').get_attribute("open") is not None and pg.input_value("#prop-expr") == formula, "it is shown under Advanced, as written")
    shot(pg, "formula")
    simple = pg.locator(".el", has=pg.locator(".pill", has_text="PREFIX"))
    ok(simple.count() == 1 and simple.locator(".fx").count() == 1, "a text joined with values and arithmetic in brackets stays a formula")
    pg.get_by_text("File", exact=True).click()
    saved = open(download(pg, lambda: pg.get_by_role("button", name="Save for JasperReports (.jrxml)").click()), encoding="utf-8").read()
    missing = [e for e in expressions(original) if e not in expressions(saved)]
    ok(not missing, f"every expression of the template is saved unchanged ({len(expressions(original))}): {missing[:3]}")

    # 9. A blank report: data, a field on the rows, "Total: {amount}".
    pg.get_by_text("File", exact=True).click()
    pg.get_by_role("button", name="Blank report").click()
    pg.wait_for_timeout(300)
    pg.locator("#data-details > summary").click()
    pg.locator("#data").fill(json.dumps([{"name": "Ana", "joined": "2024-03-01", "amount": 1200.5, "active": True}, {"name": "Luis", "joined": "2023-11-15", "amount": 990, "active": False}]))
    pg.wait_for_timeout(800)
    types = pg.locator("#fields .field-btn .type").all_inner_texts()
    ok(types == ["Text", "Date", "Number", "Yes or no"], f"field types in plain words: {types}")
    detail = pg.locator('.band-strip[data-band-key="detail:0"]')
    pg.locator(".field-btn", has_text="amount").first.drag_to(detail, target_position={"x": 40, "y": 8})
    pg.wait_for_timeout(300)
    ok("Amount" in tpl() and 'pattern="#,##0.00"' in tpl(), "a number on the rows gets a column title and a format")
    c0 = count()
    pg.locator("#prop-text").fill("Total: {amount}")
    wait_preview(c0)
    pg.get_by_text("File", exact=True).click()
    saved = open(download(pg, lambda: pg.get_by_role("button", name="Save for JasperReports (.jrxml)").click()), encoding="utf-8").read()
    ok('<expression><![CDATA["Total: " + $F{amount}]]></expression>' in saved, "Total: {amount} is saved as \"Total: \" + $F{amount}")
    pg.get_by_label("Number or date format").select_option("$#,##0.00")
    pg.wait_for_timeout(300)
    ok('pattern="$#,##0.00"' in tpl(), "a number format chosen by its example")
    if pg.locator('details[data-sec="adv"]').get_attribute("open") is None:
        pg.locator('details[data-sec="adv"] > summary').click()
    c0 = count()
    pg.get_by_label("Show only when").select_option("$F{active}")
    wait_preview(c0)
    jr = tpl()
    ok("<printWhenExpression><![CDATA[$F{active}]]></printWhenExpression>" in jr and '<field name="active" class="java.lang.Boolean"/>' in jr, "Show only when “active” is yes (and active is declared)")
    ok(not pg.locator("#stage-problem").is_visible(), "and the report is made")

    # Problems are said in plain words, with the engine's own words folded away.
    c0 = count()
    pg.locator("#prop-expr").fill("$F{amount} *")
    pg.wait_for_timeout(900)
    banner = pg.locator("#stage-problem")
    ok(banner.is_visible() and "A formula is not written correctly" in banner.locator(".problem-title").inner_text(), "a broken formula is explained in plain words: " + banner.inner_text().split("\n")[0])
    ok(not banner.locator("pre").is_visible() and "cannot parse expression" in banner.locator("pre").text_content(), "the engine's message is under Technical details")
    banner.locator("summary").click()
    ok(banner.locator("pre").is_visible(), "which opens")
    ok(pg.locator("#tab-alert").is_visible(), "the Preview tab is marked")
    shot(pg, "problem")
    pg.get_by_role("button", name="Undo (Ctrl+Z)").click()
    wait_preview(c0)
    ok(not banner.is_visible(), "undoing it takes the message away")
    pg.locator("#data").fill('[{"name": "Ana",}]')
    pg.wait_for_timeout(700)
    ok("mistake near line 1" in pg.inner_text("#data-msg"), "a mistake in the sample data says where: " + pg.inner_text("#data-msg").split("\n")[0])
    pg.context.close()

    # 10. 1366 × 768, and Spanish.
    pg = new_page(1366, 768)
    count, wait_preview = helpers(pg)
    pg.goto(BASE + "?start=invoice")
    wait_preview(0)
    pg.wait_for_timeout(500)
    zoom = int(pg.inner_text("#zoom-label").rstrip("%"))
    ok(zoom >= 75, f"at 1366 × 768 the page fits the width ({zoom}%)")
    shot(pg, "en-1366")
    side = pg.locator(".side.left").bounding_box()
    bottoms = [b.bounding_box()["y"] + b.bounding_box()["height"] for b in pg.locator("#parts button").all()]
    ok(len(bottoms) == 8 and max(bottoms) <= side["y"] + side["height"] and pg.evaluate("document.querySelector('.side.left').scrollTop") == 0, "every part to add is in sight without scrolling")
    ok(pg.locator("#data-upload-btn").is_visible() and pg.locator("#data-details > summary").is_visible(), "and so are Upload data file and Edit sample data")

    # Tab goes from a band's name to the parts on it, from the top of the page down.
    pg.locator('[data-select-band="title"]').focus()
    pg.keyboard.press("Tab")
    ok(pg.evaluate("document.activeElement.closest('[data-band-key]')?.dataset.bandKey") == "title" and pg.evaluate("document.activeElement.classList.contains('el')"), "Tab from the title's name goes to the title's text")
    pg.keyboard.press("Tab")
    ok(pg.evaluate("document.activeElement.dataset.selectBand") == "pageHeader", "then to the next band, Top of every page")
    pg.keyboard.press("Tab")
    pg.keyboard.press("Tab")
    pg.keyboard.press("Tab")
    ok(pg.evaluate("document.activeElement.closest('[data-band-key]')?.dataset.bandKey") == "detail:0", "then Column titles, the rows' name, and the parts of the rows")
    pg.keyboard.press("Escape")
    pg.locator("#file-menu > summary").click()
    pg.get_by_role("button", name="Invoice example").click()
    pg.wait_for_timeout(300)
    ok(pg.locator(".toast.show").count() == 1, "a message is on screen")
    pg.get_by_role("button", name="Español").click()
    pg.wait_for_timeout(300)
    ok(pg.locator(".toast.show").count() == 0, "switching the language takes it away")
    ok(pg.get_by_role("tab", name="Diseño").is_visible() and pg.locator(".band-label b, .band-strip b").all_inner_texts()[3] == "Cada fila de tus datos", "the interface is in Spanish")
    ok(pg.get_by_role("button", name="Descargar PDF").is_visible() and pg.inner_text("#data-h") == "Tus datos", "every part of it")
    shot(pg, "es-1366")
    pg.reload()
    wait_preview(0)
    ok(pg.evaluate("designer.lang()") == "es" and pg.inner_text("#props-h") == "Página", "and it stays Spanish after a reload")
    pg.locator('[data-band-key="detail:0"] .el', has=pg.locator(".pill", has_text="number")).first.click()
    ok(pg.locator(".psec h3").first.inner_text() == "Texto", "the panel too")
    shot(pg, "es-text-editor")
    pg.context.close()

    ok(not errors, "no errors in the console: " + "; ".join(errors))
    browser.close()

server.shutdown()
print(f"all good ({time.time() - started:.0f} s)")
