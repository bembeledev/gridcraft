//! A rich-text cell is painted run by run: each run in its own font, size and colour, the
//! unformatted runs in the cell's font; wrapped or on one line.

use std::sync::Arc;

use egui::{Color32, Shape};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::model::{Cell, Color, Font, RichText, Sheet, TextRun, Workbook};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn rich_cell(text: &str, plain: u32, red_len: u32) -> Cell {
    let red = Font { bold: true, size: 16.0, color: Color::Rgb(0xFF0000), ..Font::default() };
    let rich = RichText { runs: vec![TextRun { len: plain, font: None }, TextRun { len: red_len, font: Some(red) }] };
    Cell { rich: Some(Arc::new(rich)), ..Cell::value(Value::from(text)) }
}

fn harness(name: &str) -> Harness {
    let mut wb = Workbook::new();
    let mut sh = Sheet::new("Data");
    sh.cells.set(CellRef::parse("A1").unwrap(), rich_cell("Grade 5.5", 6, 3));
    sh.cells.set(CellRef::parse("A3").unwrap(), rich_cell("F 1 /1.00\nNote", 10, 4));
    wb.sheets = vec![Arc::new(sh)];
    let path = std::env::temp_dir().join(format!("gridcraft-rich-{}-{name}.xlsx", std::process::id()));
    std::fs::write(&path, gridcraft_xlsx::write_xlsx(&wb).unwrap()).unwrap();
    let mut s = Session::new();
    s.execute("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&path);
    s.execute("home.wrapText", json!({"range": "A3"})).unwrap();
    s.execute("home.rowHeight", json!({"rows": "3:3", "height": 60})).unwrap();
    s.execute("home.columnWidth", json!({"cols": "A:A", "chars": 20})).unwrap();
    s.execute("selection.set", json!({"range": "H20"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.run_steps(4);
    h
}

/// Underline and strikeout widths of each section of the painted text `text`.
fn decorations(h: &Harness, text: &str) -> Option<Vec<(f32, f32)>> {
    h.output().shapes.iter().find_map(|s| match &s.shape {
        Shape::Text(t) if t.galley.job.text == text => {
            Some(t.galley.job.sections.iter().map(|sec| (sec.format.underline.width, sec.format.strikethrough.width)).collect())
        }
        _ => None,
    })
}

/// (section text, colour, font size) of the painted text whose whole text is `text`.
fn sections(h: &Harness, text: &str) -> Option<Vec<(String, Color32, f32)>> {
    h.output().shapes.iter().find_map(|s| match &s.shape {
        Shape::Text(t) if t.galley.job.text == text => Some(
            t.galley
                .job
                .sections
                .iter()
                .map(|sec| {
                    (
                        t.galley.job.text.get(sec.byte_range.start.0..sec.byte_range.end.0).unwrap_or("").to_string(),
                        sec.format.color,
                        sec.format.font_id.size,
                    )
                })
                .collect(),
        ),
        _ => None,
    })
}

#[test]
fn runs_are_painted_in_their_own_font_and_colour() {
    let h = harness("one-line");
    let s = sections(&h, "Grade 5.5").expect("A1 is painted as one text");
    assert_eq!(s.len(), 2, "{s:?}");
    assert_eq!(s[0].0, "Grade ");
    assert_eq!(s[1].0, "5.5");
    assert_eq!(s[1].1, Color32::from_rgb(0xFF, 0, 0));
    assert!(s[1].2 > s[0].2, "the 16 pt run is larger than the 11 pt cell text: {s:?}");
    assert_ne!(s[0].1, s[1].1, "the first run keeps the cell's colour");
}

#[test]
fn wrapped_rich_text_keeps_its_runs() {
    let h = harness("wrapped");
    let s = sections(&h, "F 1 /1.00\nNote").expect("A3 is painted wrapped, its line break kept");
    assert_eq!(s.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), vec!["F 1 /1.00\n", "Note"]);
    assert_eq!(s[1].1, Color32::from_rgb(0xFF, 0, 0));
}

#[test]
fn the_cells_underline_and_strikeout_go_with_its_own_runs() {
    let mut h = harness("decorated");
    h.state_mut().session.execute("home.formatCells", json!({"range": "A1", "style": {"font": {"underline": "Single", "strike": true}}})).unwrap();
    h.run_steps(3);
    // "Grade " has no font of its own: the cell's underline and strikeout. "5.5" has its own font
    // without them, and no full-width line is drawn across it.
    let d = decorations(&h, "Grade 5.5").expect("A1 is painted as one text");
    assert!(d[0].0 > 0.0 && d[0].1 > 0.0, "{d:?}");
    assert_eq!(d[1], (0.0, 0.0));
    let full_width = h.output().shapes.iter().filter(|s| matches!(&s.shape, Shape::LineSegment { .. })).count();
    let mut plain = harness("decorated-plain");
    plain
        .state_mut()
        .session
        .execute("home.formatCells", json!({"range": "A1", "style": {"font": {"underline": "Single", "strike": true}}}))
        .unwrap();
    plain.state_mut().session.execute("cell.set", json!({"cell": "A1", "input": "Grade 5.5"})).unwrap();
    plain.run_steps(3);
    let plain_lines = plain.output().shapes.iter().filter(|s| matches!(&s.shape, Shape::LineSegment { .. })).count();
    assert_eq!(plain_lines, full_width + 2, "a plain cell draws its underline and strikeout as lines; a rich one doesn't");
}

#[test]
fn show_formulas_keeps_the_runs_of_constants() {
    let mut h = harness("formulas");
    h.state_mut().session.execute("formulas.showFormulas", json!({"on": true})).unwrap();
    h.run_steps(3);
    assert_eq!(sections(&h, "Grade 5.5").map(|s| s.len()), Some(2));
}

#[test]
fn a_double_underline_stays_double_in_its_runs() {
    let lines = |underline: &str| {
        let mut h = harness(&format!("underline-{underline}"));
        h.state_mut().session.execute("home.formatCells", json!({"range": "A1", "style": {"font": {"underline": underline}}})).unwrap();
        h.run_steps(3);
        let text = h
            .output()
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.job.text == "Grade 5.5" => Some((t.pos.x, t.galley.size().x)),
                _ => None,
            })
            .expect("A1 is painted as one text");
        let segments: Vec<(f32, f32)> = h
            .output()
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::LineSegment { points, .. } => Some((points[0].x, points[1].x)),
                _ => None,
            })
            .collect();
        (text, segments)
    };
    let (_, single) = lines("Single");
    let ((left, width), double) = lines("Double");
    // egui draws the first line with each run; the second is one more segment, under "Grade " only
    // (the run that takes the cell's font), not across "5.5".
    assert_eq!(double.len(), single.len() + 1, "{double:?} vs {single:?}");
    let extra: Vec<_> = double.iter().filter(|s| !single.contains(s)).collect();
    assert_eq!(extra.len(), 1, "{extra:?}");
    let (x0, x1) = *extra[0];
    assert!((x0 - left).abs() < 1.0 && x1 < left + width - 5.0, "the second line spans \"Grade \": {x0}..{x1} in {left}+{width}");
}
