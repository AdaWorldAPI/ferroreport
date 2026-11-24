//! Right-to-left and complex scripts: shaping, bidirectional order, alignment as
//! JasperReports does it, and simple scripts left exactly as they were.

use ferroreport::print::{PrintItem, PrintLine};
use ferroreport::Renderer;

fn report(body: &str, font: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<jasperReport xmlns="http://jasperreports.sourceforge.net/jasperreports" name="shaping" whenNoDataType="AllSectionsNoDetail" pageWidth="420" pageHeight="400" columnWidth="400" leftMargin="10" rightMargin="10" topMargin="10" bottomMargin="10">
<style name="B" isDefault="true" fontName="{font}" fontSize="12"/>
<title><band height="300">{body}</band></title>
</jasperReport>"#
    )
}

fn text(y: u32, w: u32, align: &str, s: &str) -> String {
    format!(
        r#"<staticText><reportElement x="0" y="{y}" width="{w}" height="60"/><textElement textAlignment="{align}"/><text><![CDATA[{s}]]></text></staticText>"#
    )
}

/// The lines of every text drawn, element by element.
fn lines(renderer: &mut Renderer, template: &str) -> Vec<Vec<PrintLine>> {
    fn walk(items: &[PrintItem], out: &mut Vec<Vec<PrintLine>>) {
        for item in items {
            match item {
                PrintItem::Text(t) => out.push(t.lines.clone()),
                PrintItem::Cell(_, inner) => walk(std::slice::from_ref(inner), out),
                PrintItem::Group(items) | PrintItem::Transform(_, items) | PrintItem::Paragraphs(_, items) => walk(items, out),
                _ => {}
            }
        }
    }
    let report = renderer.parse(template).unwrap();
    let doc = renderer.fill(&report, &[], Default::default()).unwrap();
    let mut out = Vec::new();
    for page in &doc.pages {
        walk(&page.items, &mut out);
    }
    out
}

#[test]
fn latin_text_is_not_shaped() {
    let mut r = Renderer::new();
    let body = text(0, 200, "Left", "Plain Latin text, wrapped over a few lines — ¿qué tal? Ελληνικά Кириллица");
    for element in lines(&mut r, &report(&body, "DejaVu Sans")) {
        assert!(element.iter().all(|l| l.shaped.is_none()));
    }
}

#[test]
fn right_to_left_text_keeps_jasper_alignment() {
    let mut r = Renderer::new();
    let body = [
        text(0, 400, "Left", "שלום עולם 123 abc"),
        text(60, 400, "Right", "שלום עולם 123 abc"),
        text(120, 120, "Justified", "זהו טקסט עברי ארוך שנשבר לכמה שורות בתוך התיבה"),
        text(180, 400, "Left", "Hello שלום world"),
    ]
    .concat();
    let all = lines(&mut r, &report(&body, "DejaVu Sans"));
    // Left is left and right is right, whatever the direction (as JasperReports draws it).
    let left = &all[0][0];
    assert!(left.shaped.as_ref().unwrap().rtl);
    assert!((left.x - 10.0).abs() < 1e-3, "{}", left.x);
    let right = &all[1][0];
    assert!((right.x + right.width - 410.0).abs() < 1e-3);
    // Justified: full lines fill the box, the paragraph's last line sits on the right.
    let justified = &all[2];
    assert!(justified.len() > 1);
    let first = &justified[0];
    assert!((first.x - 10.0).abs() < 1e-2 && (first.width - 120.0).abs() < 0.5, "{first:?}");
    let last = justified.last().unwrap();
    assert!((last.x + last.width - 130.0).abs() < 1e-2, "{last:?}");
    // A paragraph that starts left to right runs left to right.
    assert!(!all[3][0].shaped.as_ref().unwrap().rtl);
}

#[test]
fn arabic_wraps_at_spaces_and_joins() {
    let mut r = Renderer::new();
    let body = text(0, 120, "Left", "هذا نص عربي طويل يلتف على عدة أسطر في الصندوق");
    let all = lines(&mut r, &report(&body, "DejaVu Sans"));
    let element = &all[0];
    assert!(element.len() >= 3, "{element:?}");
    for l in element {
        let shaped = l.shaped.as_ref().unwrap();
        assert!(l.width <= 120.0 + 1e-3);
        // Joined letters take contextual forms, not the nominal glyph of the character.
        assert!(shaped.glyphs.len() <= l.text.chars().count());
    }
    let rejoined: Vec<&str> = element.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(rejoined.join(" "), "هذا نص عربي طويل يلتف على عدة أسطر في الصندوق");
}

#[test]
fn html_says_which_lines_run_right_to_left() {
    let mut r = Renderer::new();
    let template = report(&text(0, 400, "Left", "مرحبا بالعالم 2024"), "DejaVu Sans");
    let report = r.parse(&template).unwrap();
    let doc = r.fill(&report, &[], Default::default()).unwrap();
    let html = String::from_utf8(r.export(&doc, ferroreport::Format::Html).unwrap()).unwrap();
    assert!(html.contains("direction=\"rtl\""));
    assert!(html.contains("مرحبا بالعالم 2024"));
}

/// Devanagari and Thai need a font that has them; the bundled DejaVu does not. GNU FreeSerif
/// (fonts-freefont-ttf) is used when it is installed.
fn free_serif() -> Option<Vec<u8>> {
    std::fs::read("/usr/share/fonts/truetype/freefont/FreeSerif.ttf").ok()
}

#[test]
fn devanagari_and_thai_are_shaped_and_broken() {
    let Some(font) = free_serif() else {
        eprintln!("FreeSerif is not installed: skipped");
        return;
    };
    let mut r = Renderer::new();
    r.register_font("FreeSerif", false, false, font).unwrap();
    let body = [text(0, 400, "Left", "हिन्दी क्ष श्री"), text(60, 120, "Left", "สวัสดีครับเราไปโรงเรียนด้วยกันในวันจันทร์")].concat();
    let all = lines(&mut r, &report(&body, "FreeSerif"));
    let hindi = &all[0][0];
    let shaped = hindi.shaped.as_ref().unwrap();
    assert!(!shaped.rtl);
    // Conjuncts make fewer glyphs than characters; the vowel sign ि of हि is drawn first.
    assert!(shaped.glyphs.len() < hindi.text.chars().count());
    let first = shaped.clusters(&hindi.text).next().unwrap();
    assert_eq!(first.0, "हि");
    assert_eq!(first.1.len(), 2);
    // Thai has no spaces: lines break between syllables, never inside a cluster.
    let thai = &all[1];
    assert!(thai.len() > 1, "{thai:?}");
    for l in thai {
        assert!(l.width <= 120.0 + 1e-3);
        let c = l.text.chars().next().unwrap();
        // No line starts with a combining vowel or tone mark.
        assert!(!matches!(c as u32, 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E), "{}", l.text);
    }
    assert_eq!(thai.iter().map(|l| l.text.as_str()).collect::<String>(), "สวัสดีครับเราไปโรงเรียนด้วยกันในวันจันทร์");
}

/// Characters a font lacks are drawn in another font only when asked (`font_fallback`), as a
/// JasperReports font set does: the longest run one font has, in the first font that has it.
#[test]
fn missing_characters_fall_back_to_another_font() {
    let template = report(&text(0, 400, "Left", "Serif ⤴ then ★ star, عربي"), "DejaVu Serif");
    let pdf = |r: &mut Renderer| {
        let report = r.parse(&template).unwrap();
        let doc = r.fill(&report, &[], Default::default()).unwrap();
        r.to_pdf(&doc).unwrap()
    };
    let has = |pdf: &[u8], name: &[u8]| pdf.windows(name.len()).any(|w| w == name);
    let mut plain = Renderer::new();
    assert_eq!(lines(&mut plain, &template).len(), 1);
    assert!(!has(&pdf(&mut plain), b"DejaVuSans"));
    let mut fallback = Renderer::new().font_fallback(true);
    let runs: Vec<String> = lines(&mut fallback, &template).iter().flatten().map(|l| l.text.clone()).collect();
    // DejaVu Serif has the arrow, DejaVu Sans the star and the Arabic letters (shaped).
    assert_eq!(runs[0], "Serif ⤴ then ");
    assert_eq!(runs[1..].concat(), "★ star, عربي");
    assert!(has(&pdf(&mut fallback), b"DejaVuSans"));
}
