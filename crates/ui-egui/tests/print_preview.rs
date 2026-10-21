//! The File ▸ Print thumbnail draws cell orientation as the worksheet grid does: stacked vertical
//! text (`textRotation="255"`) as upright glyphs one below another, and ordinary angles as one
//! turned galley. Before this, every cell was painted as a single horizontal run.

use egui::{Pos2, Rect, Shape, vec2};
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

/// One painted text run: its string, where egui put it, its angle in degrees, and the rectangle it
/// is clipped to.
#[derive(Clone, Debug)]
struct Run {
    text: String,
    pos: Pos2,
    degrees: f32,
    clip: Rect,
}

/// Tall rows (2:6) fit the whole B2 stack; everything else is a default-height row.
fn fixture() -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("home.rowHeight", json!({"rows": "2:6", "height": 120.0})).unwrap();
    s.execute("cell.set", json!({"cell": "A1", "input": "PLAIN"})).unwrap();
    // B2 stacked vertical; B3 90°; B4 45° — the reproduction fixture's three modes.
    s.execute("cell.set", json!({"cell": "B2", "input": "ZQXJV"})).unwrap();
    s.execute("home.orientation", json!({"range": "B2", "angle": "vertical"})).unwrap();
    s.execute("cell.set", json!({"cell": "B3", "input": "QR90"})).unwrap();
    s.execute("home.orientation", json!({"range": "B3", "angle": 90})).unwrap();
    s.execute("cell.set", json!({"cell": "B4", "input": "QR45"})).unwrap();
    s.execute("home.orientation", json!({"range": "B4", "angle": 45})).unwrap();
    // General alignment, rotated: a number on the right, text on the left (same column, so the
    // cells have the same width).
    s.execute("cell.set", json!({"cell": "B5", "input": 2024})).unwrap();
    s.execute("home.orientation", json!({"range": "B5", "angle": 90})).unwrap();
    s.execute("cell.set", json!({"cell": "B6", "input": "GEN90"})).unwrap();
    s.execute("home.orientation", json!({"range": "B6", "angle": 90})).unwrap();
    // A stack in a default-height row: the rest is clipped at the row's bottom, never drawn sideways.
    s.execute("cell.set", json!({"cell": "C7", "input": "CLIPME"})).unwrap();
    s.execute("home.orientation", json!({"range": "C7", "angle": "vertical"})).unwrap();
    // Horizontal regression: a number, a text cell, a neighbour-blocked cell and an overflowing one.
    s.execute("cell.set", json!({"cell": "A8", "input": 12345})).unwrap();
    s.execute("cell.set", json!({"cell": "A9", "input": "LEFTY"})).unwrap();
    s.execute("cell.set", json!({"cell": "A3", "input": "SHORT"})).unwrap();
    s.execute("cell.set", json!({"cell": "A10", "input": "OVERFLOWTEXT"})).unwrap();
    let mut app = SheetApp::new(s, Default::default());
    app.run("ui.backstage", json!({"page": "print"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1100.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(6);
    h
}

fn runs(h: &Harness) -> Vec<Run> {
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            Shape::Text(t) => Some(Run { text: t.galley.job.text.clone(), pos: t.pos, degrees: t.angle.to_degrees(), clip: s.clip_rect }),
            _ => None,
        })
        .collect()
}

/// The page thumbnail's rectangle, found by its size (the preview allocates exactly the paper's
/// proportions; the drop shadow is the same size, so the top-left-most match is the page itself).
fn preview_page(h: &mut Harness) -> Rect {
    let pv = h.state_mut().session.run("file.printPreview", json!({})).unwrap();
    let page = &pv["pageList"][0];
    let w = page["width"].as_f64().unwrap_or(612.0) as f32;
    let ht = page["height"].as_f64().unwrap_or(792.0) as f32;
    let scale = (520.0 / ht).min(440.0 / w);
    let size = vec2(w * scale, ht * scale);
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            Shape::Rect(r) if (r.rect.width() - size.x).abs() < 1.5 && (r.rect.height() - size.y).abs() < 1.5 => Some(r.rect),
            _ => None,
        })
        .min_by(|a, b| (a.min.x + a.min.y).total_cmp(&(b.min.x + b.min.y)))
        .expect("the page thumbnail is drawn")
}

fn inside(outer: Rect, inner: Rect) -> bool {
    const E: f32 = 1.5;
    inner.min.x >= outer.min.x - E && inner.min.y >= outer.min.y - E && inner.max.x <= outer.max.x + E && inner.max.y <= outer.max.y + E
}

/// B2: one upright glyph per character, in reading order, all at one x and growing downwards —
/// and never the whole string as one horizontal run.
#[test]
fn a_vertical_cell_is_stacked_upright_in_the_print_preview() {
    let mut h = fixture();
    let page = preview_page(&mut h);
    let all = runs(&h);
    let mut stack: Vec<&Run> =
        all.iter().filter(|r| r.text.chars().count() == 1 && "ZQXJV".contains(r.text.as_str()) && page.contains(r.pos)).collect();
    stack.sort_by(|a, b| a.pos.y.total_cmp(&b.pos.y));
    assert_eq!(stack.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["Z", "Q", "X", "J", "V"], "one glyph per character, reading down");
    assert!(stack.iter().all(|r| r.degrees.abs() < 1e-3), "no glyph is turned: {:?}", stack.iter().map(|r| r.degrees).collect::<Vec<_>>());
    assert!(
        stack.windows(2).all(|w| (w[1].pos.x - w[0].pos.x).abs() < 0.5),
        "the stack keeps one straight column: {:?}",
        stack.iter().map(|r| r.pos.x).collect::<Vec<_>>()
    );
    assert!(stack.windows(2).all(|w| w[1].pos.y > w[0].pos.y), "the stack grows downwards");
    assert!(all.iter().all(|r| r.text != "ZQXJV"), "the string is never painted as one sideways run");
}

/// B3 and B4: one whole galley turned by the grid's convention (-90° and -45°), not split into
/// stacked characters.
#[test]
fn ordinary_rotation_turns_the_print_preview_text_as_the_grid_does() {
    let h = fixture();
    let all = runs(&h);
    for (text, degrees) in [("QR90", 90.0), ("QR45", 45.0)] {
        let run = all.iter().find(|r| r.text == text).unwrap_or_else(|| panic!("{text} is drawn"));
        assert!((run.degrees + degrees).abs() < 1e-3, "{text} is turned {degrees}° down, got {}", run.degrees);
        // Digits are only in these two strings, so a lone digit proves a sideways fallback.
        assert!(
            !all.iter().any(|r| r.text.chars().count() == 1 && ("9045".contains(r.text.as_str()))),
            "{text} is not drawn as separate stacked characters"
        );
    }
}

/// Unrotated cells keep their old path: one run each, General numbers right of General text, and
/// text still running on over an empty neighbour.
#[test]
fn horizontal_print_preview_text_keeps_its_alignment_and_overflow() {
    let mut h = fixture();
    let page = preview_page(&mut h);
    let all = runs(&h);
    let plain = all.iter().find(|r| r.text == "PLAIN").expect("horizontal text is one run");
    assert!(plain.degrees.abs() < 1e-3);
    assert!(inside(page, plain.clip), "the run stays on the page");
    let number = all.iter().find(|r| r.text == "12345").expect("the number is drawn");
    let text = all.iter().find(|r| r.text == "LEFTY").expect("the text is drawn");
    assert!(number.degrees.abs() < 1e-3 && text.degrees.abs() < 1e-3);
    assert!(number.pos.x > text.pos.x, "a General number sits right of General text: {} vs {}", number.pos.x, text.pos.x);
    // A filled neighbour stops the overflow; an empty one lets the clip run on.
    let blocked = all.iter().find(|r| r.text == "SHORT").expect("SHORT is drawn");
    let over = all.iter().find(|r| r.text == "OVERFLOWTEXT").expect("OVERFLOWTEXT is drawn");
    assert!(over.clip.width() > blocked.clip.width() * 1.5, "text runs on over empty neighbours: {} vs {}", over.clip.width(), blocked.clip.width());
}

/// Oriented text is clipped to the cell and the printable body: a default-height row shows only the
/// glyphs that fit, and every oriented run is drawn inside the page thumbnail.
#[test]
fn oriented_print_preview_text_is_clipped_to_the_cell_and_the_page() {
    let mut h = fixture();
    let page = preview_page(&mut h);
    let all = runs(&h);
    let clipped: Vec<&Run> = all.iter().filter(|r| r.text.chars().count() == 1 && "CLIPME".contains(r.text.as_str())).collect();
    assert!(!clipped.is_empty(), "at least the first glyph fits");
    assert!(
        clipped.len() < "CLIPME".chars().count(),
        "a short row clips the rest instead of drawing them sideways: {:?}",
        clipped.iter().map(|r| r.text.as_str()).collect::<Vec<_>>()
    );
    let oriented = |r: &Run| {
        r.degrees.abs() > 1e-3 || (r.text.chars().count() == 1 && ("ZQXJV".contains(r.text.as_str()) || "CLIPME".contains(r.text.as_str())))
    };
    for r in all.iter().filter(|r| oriented(r)) {
        assert!(inside(page, r.clip), "oriented text is clipped to a rectangle on the page: {:?} {:?} vs {:?}", r.text, r.clip, page);
        assert!(r.clip.is_positive(), "the clip rectangle is real: {:?}", r.clip);
    }
}

/// General alignment resolves by value type for rotated text too: a number right, text left. (The
/// grid resolves General the same way for its own runs; it passes the raw alignment to
/// `rotated_text_pos`, which keeps General at the left — a deliberate improvement here, per task.)
#[test]
fn general_alignment_resolves_by_value_for_rotated_preview_text() {
    let h = fixture();
    let all = runs(&h);
    let number = all.iter().find(|r| r.text == "2024").expect("the rotated number is drawn");
    let text = all.iter().find(|r| r.text == "GEN90").expect("the rotated text is drawn");
    assert!((number.degrees + 90.0).abs() < 1e-3 && (text.degrees + 90.0).abs() < 1e-3, "both are turned 90° down");
    assert!(number.pos.x > text.pos.x + 5.0, "General + number is right-aligned, General + text is left: {} vs {}", number.pos.x, text.pos.x);
}
