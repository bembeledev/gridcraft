//! Ribbon artwork for the desktop-style tabs.
//!
//! Every icon is drawn in code from simple shapes, in the flat two-tone style of desktop
//! spreadsheet ribbons: a dark outline in the ink colour with one accent colour. The shapes are
//! this project's own; none is copied from another product's image files.

use egui::{Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

const BLUE: Color32 = Color32::from_rgb(0x2B, 0x7C, 0xD3);
const GREEN: Color32 = Color32::from_rgb(0x21, 0xA3, 0x66);
const DARK_GREEN: Color32 = Color32::from_rgb(0x10, 0x7C, 0x41);
const RED: Color32 = Color32::from_rgb(0xD1, 0x34, 0x38);
const ORANGE: Color32 = Color32::from_rgb(0xE8, 0x91, 0x2D);
const YELLOW: Color32 = Color32::from_rgb(0xF2, 0xC8, 0x11);
const PURPLE: Color32 = Color32::from_rgb(0x87, 0x64, 0xB8);
const TEAL: Color32 = Color32::from_rgb(0x03, 0x83, 0x87);
const GRAY: Color32 = Color32::from_rgb(0x9A, 0x9A, 0x9A);
const PAPER: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);

/// The icons of the Insert, Draw, Formulas, Data, Review, View, Automate, Developer and Help tabs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Art {
    // Insert
    PivotTable,
    RecommendedPivot,
    Table,
    Forms,
    Pictures,
    Shapes,
    Icons,
    Models3d,
    SmartArt,
    Screenshot,
    Checkbox,
    RecommendedCharts,
    ChartColumn,
    ChartLine,
    ChartPie,
    ChartHierarchy,
    ChartStatistic,
    ChartCombo,
    ChartWaterfall,
    ChartScatter,
    Maps,
    PivotChart,
    SparkLine,
    SparkColumn,
    SparkWinLoss,
    Slicer,
    Timeline,
    Link,
    Comment,
    TextBox,
    HeaderFooter,
    WordArt,
    SignatureLine,
    Object,
    Equation,
    Symbol,
    // Draw
    Undo,
    Redo,
    Select,
    Lasso,
    Eraser,
    Pen(Color32),
    Highlighter(Color32),
    Pencil(Color32),
    GalaxyPen,
    AddPen,
    InkToShape,
    InkToMath,
    InkReplay,
    Help,
    // Formulas
    InsertFunction,
    AutoSum,
    RecentlyUsed,
    Financial,
    Logical,
    Text,
    DateTime,
    Lookup,
    MathTrig,
    MoreFunctions,
    Python,
    Reset,
    Editor,
    Initialization,
    NameManager,
    DefineName,
    UseInFormula,
    CreateFromSelection,
    TracePrecedents,
    TraceDependents,
    RemoveArrows,
    ShowFormulas,
    ErrorChecking,
    EvaluateFormula,
    WatchWindow,
    CalcOptions,
    CalcNow,
    CalcSheet,
    // Data
    GetData,
    FromCsv,
    FromWeb,
    FromTable,
    FromPicture,
    RecentSources,
    ExistingConnections,
    RefreshAll,
    Queries,
    Properties,
    WorkbookLinks,
    Stocks,
    Currencies,
    SortAz,
    SortZa,
    Sort,
    Filter,
    ClearFilter,
    Reapply,
    Advanced,
    TextToColumns,
    FlashFill,
    RemoveDuplicates,
    DataValidation,
    Consolidate,
    DataModel,
    AnalyzeData,
    WhatIf,
    ForecastSheet,
    Group,
    Ungroup,
    Subtotal,
    ShowDetail,
    HideDetail,
    // Review
    Spelling,
    Thesaurus,
    WorkbookStats,
    CheckPerformance,
    Accessibility,
    Translate,
    ShowChanges,
    NewComment,
    DeleteComment,
    PreviousComment,
    NextComment,
    ShowComments,
    Notes,
    ProtectSheet,
    ProtectWorkbook,
    AllowEditRanges,
    UnshareWorkbook,
    HideInk,
    // View
    Keep,
    Exit,
    NewView,
    ViewOptions,
    Normal,
    PageBreakPreview,
    PageLayoutView,
    CustomViews,
    Navigation,
    FocusCell,
    Zoom,
    Zoom100,
    ZoomSelection,
    NewWindow,
    ArrangeAll,
    FreezePanes,
    Split,
    HideWindow,
    UnhideWindow,
    SideBySide,
    SyncScroll,
    ResetPosition,
    SwitchWindows,
    Macros,
    // Automate and Developer
    NewScript,
    ViewScripts,
    ScriptItem,
    AutomationTemplates,
    VisualBasic,
    RecordMacro,
    RelativeRefs,
    MacroSecurity,
    AddIns,
    ExcelAddIns,
    ComAddIns,
    InsertControl,
    DesignMode,
    ControlProperties,
    ViewCode,
    RunDialog,
    XmlSource,
    MapProperties,
    ExpansionPacks,
    RefreshData,
    Import,
    Export,
    Palette,
    Journal,
    Agent,
    About,
    // Help
    ContactSupport,
    Feedback,
    ShowTraining,
    WhatsNew,
    Community,
    Blog,
    MobileApp,
    PdfXps,
}

/// Unit coordinates (0..1 across `r`) to a point.
fn u(r: Rect, x: f32, y: f32) -> Pos2 {
    pos2(r.left() + x * r.width(), r.top() + y * r.height())
}

/// A sub-rectangle in unit coordinates.
fn ur(r: Rect, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
    Rect::from_min_max(u(r, x0, y0), u(r, x1, y1))
}

fn line(p: &Painter, a: Pos2, b: Pos2, s: Stroke) {
    p.line_segment([a, b], s);
}

fn poly(p: &Painter, pts: Vec<Pos2>, fill: Color32, s: Stroke) {
    p.add(Shape::convex_polygon(pts, fill, s));
}

fn path(p: &Painter, pts: Vec<Pos2>, s: Stroke) {
    p.add(Shape::line(pts, s));
}

/// A worksheet: a white frame with a header band and grid lines.
fn sheet(p: &Painter, r: Rect, ink: Color32, band: Color32, rows: usize, cols: usize) {
    let w = (r.width() / 22.0).max(1.0);
    p.rect_filled(r, CornerRadius::same(1), PAPER);
    p.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), r.height() / (rows as f32 + 1.0))), CornerRadius::same(1), band);
    let s = Stroke::new(w, ink);
    for i in 1..=rows {
        let y = r.top() + r.height() * i as f32 / (rows as f32 + 1.0);
        p.hline(r.x_range(), y, Stroke::new(w * 0.7, GRAY));
    }
    for j in 1..cols {
        let x = r.left() + r.width() * j as f32 / cols as f32;
        p.vline(x, r.y_range(), Stroke::new(w * 0.7, GRAY));
    }
    p.rect_stroke(r, CornerRadius::same(1), s, StrokeKind::Inside);
}

/// A page with a folded corner.
fn page(p: &Painter, r: Rect, ink: Color32) {
    let w = (r.width() / 22.0).max(1.0);
    let f = r.width() * 0.28;
    let pts = vec![r.left_top(), pos2(r.right() - f, r.top()), pos2(r.right(), r.top() + f), r.right_bottom(), r.left_bottom()];
    poly(p, pts.clone(), PAPER, Stroke::NONE);
    let mut outline = pts;
    outline.push(r.left_top());
    path(p, outline, Stroke::new(w, ink));
    path(p, vec![pos2(r.right() - f, r.top()), pos2(r.right() - f, r.top() + f), pos2(r.right(), r.top() + f)], Stroke::new(w, ink));
}

/// Text lines on a page or card.
fn lines(p: &Painter, r: Rect, n: usize, color: Color32) {
    let w = (r.height() / 14.0).max(1.0);
    for i in 0..n {
        let y = r.top() + r.height() * (i as f32 + 0.5) / n as f32;
        let x1 = if i + 1 == n { r.left() + r.width() * 0.6 } else { r.right() };
        p.hline(r.left()..=x1, y, Stroke::new(w, color));
    }
}

/// A speech bubble.
fn bubble(p: &Painter, r: Rect, ink: Color32, fill: Color32) {
    let w = (r.width() / 20.0).max(1.0);
    let body = ur(r, 0.0, 0.0, 1.0, 0.72);
    p.rect_filled(body, CornerRadius::same(2), fill);
    p.rect_stroke(body, CornerRadius::same(2), Stroke::new(w, ink), StrokeKind::Inside);
    poly(p, vec![u(r, 0.22, 0.70), u(r, 0.22, 0.98), u(r, 0.48, 0.70)], fill, Stroke::NONE);
    path(p, vec![u(r, 0.22, 0.71), u(r, 0.22, 0.98), u(r, 0.48, 0.71)], Stroke::new(w, ink));
}

/// A padlock in the lower-right corner of `r`.
fn padlock(p: &Painter, r: Rect) {
    let body = ur(r, 0.52, 0.62, 0.98, 1.0);
    p.circle_stroke(pos2(body.center().x, body.top()), body.width() * 0.28, Stroke::new((r.width() / 16.0).max(1.2), Color32::from_gray(90)));
    p.rect_filled(body, CornerRadius::same(2), ORANGE);
}

/// A magnifier.
fn magnifier(p: &Painter, c: Pos2, rad: f32, ink: Color32) {
    let w = (rad / 3.5).max(1.2);
    p.circle_filled(c, rad, PAPER);
    p.circle_stroke(c, rad, Stroke::new(w, ink));
    let d = rad * 0.72;
    line(p, pos2(c.x + d, c.y + d), pos2(c.x + rad * 1.9, c.y + rad * 1.9), Stroke::new(w * 1.4, ink));
}

/// A small funnel.
fn funnel(p: &Painter, r: Rect, ink: Color32, fill: Color32) {
    let w = (r.width() / 18.0).max(1.0);
    let pts = vec![u(r, 0.05, 0.08), u(r, 0.95, 0.08), u(r, 0.58, 0.52), u(r, 0.58, 0.95), u(r, 0.42, 0.82), u(r, 0.42, 0.52)];
    p.add(Shape::convex_polygon(vec![u(r, 0.05, 0.08), u(r, 0.95, 0.08), u(r, 0.58, 0.52), u(r, 0.42, 0.52)], fill, Stroke::NONE));
    p.rect_filled(ur(r, 0.42, 0.5, 0.58, 0.9), CornerRadius::ZERO, fill);
    let mut outline = pts;
    outline.push(u(r, 0.05, 0.08));
    path(p, outline, Stroke::new(w, ink));
}

/// Column bars with the given heights (0..1) and colour.
fn bars(p: &Painter, r: Rect, heights: &[f32], color: Color32) {
    let n = heights.len() as f32;
    let bw = r.width() / (n * 1.6);
    for (i, h) in heights.iter().enumerate() {
        let x = r.left() + (i as f32 + 0.3) * r.width() / n;
        p.rect_filled(Rect::from_min_max(pos2(x, r.bottom() - h * r.height()), pos2(x + bw, r.bottom())), CornerRadius::ZERO, color);
    }
}

/// A text glyph centred in `r`.
fn glyph(p: &Painter, r: Rect, text: &str, scale: f32, color: Color32) {
    p.text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(r.height() * scale), color);
}

/// A pen nib pointing down: `body` colour, a tip of the same colour.
fn pen(p: &Painter, r: Rect, color: Color32, highlighter: bool) {
    let w = (r.width() / 18.0).max(1.0);
    let ink = Color32::from_gray(40);
    let body = ur(r, 0.3, 0.0, 0.7, 0.6);
    p.rect_filled(body, CornerRadius::same(2), if highlighter { color } else { Color32::from_gray(30) });
    p.rect_stroke(body, CornerRadius::same(2), Stroke::new(w, ink), StrokeKind::Inside);
    if !highlighter {
        p.rect_filled(ur(r, 0.3, 0.12, 0.7, 0.24), CornerRadius::ZERO, color);
    }
    let cone = vec![u(r, 0.3, 0.6), u(r, 0.7, 0.6), u(r, 0.55, 0.9), u(r, 0.45, 0.9)];
    poly(p, cone, PAPER, Stroke::new(w, ink));
    poly(p, vec![u(r, 0.44, 0.88), u(r, 0.56, 0.88), u(r, 0.5, 1.0)], color, Stroke::NONE);
}

/// Draws `art` in `r` with `ink` as the outline colour (dimmed when disabled).
pub fn paint(p: &Painter, r: Rect, art: Art, ink: Color32) {
    let w = (r.width() / 20.0).max(1.0);
    let s = Stroke::new(w, ink);
    match art {
        // ------------------------------------------------------------------ Insert
        Art::PivotTable => {
            sheet(p, ur(r, 0.05, 0.05, 0.85, 0.95), ink, Color32::from_gray(225), 4, 3);
            let a = ur(r, 0.45, 0.45, 0.95, 0.95);
            path(p, vec![a.left_top(), pos2(a.right(), a.top()), a.right_bottom()], Stroke::new(w * 1.2, BLUE));
            poly(p, vec![pos2(a.right() - 3.0, a.bottom() - 4.0), pos2(a.right() + 3.0, a.bottom() - 4.0), a.right_bottom()], BLUE, Stroke::NONE);
        }
        Art::RecommendedPivot => {
            sheet(p, ur(r, 0.05, 0.05, 0.8, 0.95), ink, Color32::from_gray(225), 4, 3);
            glyph(p, ur(r, 0.5, 0.0, 1.0, 0.6), "?", 1.0, BLUE);
        }
        Art::Table => sheet(p, ur(r, 0.08, 0.1, 0.92, 0.9), ink, BLUE, 4, 3),
        Art::Forms => {
            p.rect_filled(ur(r, 0.08, 0.08, 0.92, 0.92), CornerRadius::same(3), TEAL);
            glyph(p, ur(r, 0.08, 0.08, 0.92, 0.92), "F", 0.7, PAPER);
        }
        Art::Pictures => {
            let f = ur(r, 0.05, 0.12, 0.95, 0.88);
            p.rect_filled(f, CornerRadius::same(1), PAPER);
            poly(p, vec![u(r, 0.12, 0.82), u(r, 0.42, 0.42), u(r, 0.66, 0.82)], BLUE, Stroke::NONE);
            poly(p, vec![u(r, 0.48, 0.82), u(r, 0.7, 0.55), u(r, 0.9, 0.82)], DARK_GREEN, Stroke::NONE);
            p.circle_filled(u(r, 0.75, 0.32), r.width() * 0.08, YELLOW);
            p.rect_stroke(f, CornerRadius::same(1), s, StrokeKind::Inside);
        }
        Art::Shapes => {
            p.circle_stroke(u(r, 0.35, 0.35), r.width() * 0.26, Stroke::new(w, BLUE));
            p.rect_filled(ur(r, 0.4, 0.4, 0.92, 0.92), CornerRadius::ZERO, PAPER);
            p.rect_stroke(ur(r, 0.4, 0.4, 0.92, 0.92), CornerRadius::ZERO, s, StrokeKind::Inside);
        }
        Art::Icons => {
            p.circle_stroke(u(r, 0.5, 0.5), r.width() * 0.4, s);
            p.circle_filled(u(r, 0.36, 0.42), r.width() * 0.05, ink);
            p.circle_filled(u(r, 0.64, 0.42), r.width() * 0.05, ink);
            path(p, vec![u(r, 0.32, 0.6), u(r, 0.5, 0.72), u(r, 0.68, 0.6)], Stroke::new(w, BLUE));
        }
        Art::Models3d => {
            let pts = [u(r, 0.5, 0.05), u(r, 0.92, 0.28), u(r, 0.92, 0.75), u(r, 0.5, 0.98), u(r, 0.08, 0.75), u(r, 0.08, 0.28)];
            poly(p, vec![pts[0], pts[1], u(r, 0.5, 0.5), pts[5]], Color32::from_rgb(0xB4, 0xD3, 0xF2), Stroke::NONE);
            for i in 0..6 {
                line(p, pts[i], pts[(i + 1) % 6], Stroke::new(w, BLUE));
            }
            line(p, u(r, 0.5, 0.5), pts[1], Stroke::new(w, BLUE));
            line(p, u(r, 0.5, 0.5), pts[5], Stroke::new(w, BLUE));
            line(p, u(r, 0.5, 0.5), pts[3], Stroke::new(w, BLUE));
        }
        Art::SmartArt => {
            for (x, y) in [(0.1, 0.55), (0.4, 0.15), (0.62, 0.55)] {
                p.rect_filled(ur(r, x, y, x + 0.3, y + 0.3), CornerRadius::same(1), DARK_GREEN);
            }
            path(p, vec![u(r, 0.25, 0.55), u(r, 0.25, 0.45), u(r, 0.77, 0.45), u(r, 0.77, 0.55)], s);
        }
        Art::Screenshot => {
            let f = ur(r, 0.05, 0.15, 0.95, 0.85);
            p.rect_stroke(f, CornerRadius::same(1), Stroke { width: w, color: ink }, StrokeKind::Inside);
            p.rect_stroke(ur(r, 0.3, 0.35, 0.75, 0.7), CornerRadius::ZERO, Stroke::new(w, BLUE), StrokeKind::Inside);
            line(p, u(r, 0.22, 0.35), u(r, 0.3, 0.35), Stroke::new(w, BLUE));
        }
        Art::Checkbox => {
            p.rect_stroke(ur(r, 0.1, 0.1, 0.9, 0.9), CornerRadius::same(2), s, StrokeKind::Inside);
            path(p, vec![u(r, 0.25, 0.5), u(r, 0.43, 0.7), u(r, 0.78, 0.28)], Stroke::new(w * 1.6, GREEN));
        }
        Art::RecommendedCharts => {
            p.rect_stroke(ur(r, 0.05, 0.1, 0.8, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            bars(p, ur(r, 0.12, 0.35, 0.72, 0.88), &[0.5, 0.9, 0.65], GRAY);
            glyph(p, ur(r, 0.55, 0.0, 1.0, 0.5), "?", 1.0, BLUE);
        }
        Art::ChartColumn => bars(p, ur(r, 0.08, 0.1, 0.92, 0.92), &[0.55, 1.0, 0.75], BLUE),
        Art::ChartLine => {
            path(p, vec![u(r, 0.08, 0.8), u(r, 0.35, 0.45), u(r, 0.6, 0.65), u(r, 0.92, 0.2)], Stroke::new(w * 1.4, BLUE));
            path(p, vec![u(r, 0.08, 0.92), u(r, 0.92, 0.92)], s);
        }
        Art::ChartPie => {
            p.circle_filled(u(r, 0.5, 0.5), r.width() * 0.42, BLUE);
            poly(p, vec![u(r, 0.5, 0.5), u(r, 0.5, 0.08), u(r, 0.82, 0.18), u(r, 0.92, 0.5)], ORANGE, Stroke::NONE);
        }
        Art::ChartHierarchy => {
            p.rect_filled(ur(r, 0.05, 0.1, 0.55, 0.9), CornerRadius::ZERO, BLUE);
            p.rect_filled(ur(r, 0.58, 0.1, 0.95, 0.5), CornerRadius::ZERO, ORANGE);
            p.rect_filled(ur(r, 0.58, 0.53, 0.95, 0.9), CornerRadius::ZERO, GRAY);
        }
        Art::ChartStatistic => {
            bars(p, ur(r, 0.08, 0.15, 0.92, 0.92), &[0.35, 0.7, 1.0, 0.6, 0.3], BLUE);
        }
        Art::ChartCombo => {
            bars(p, ur(r, 0.08, 0.3, 0.92, 0.92), &[0.6, 0.9, 0.5], BLUE);
            path(p, vec![u(r, 0.12, 0.35), u(r, 0.5, 0.15), u(r, 0.88, 0.3)], Stroke::new(w * 1.4, ORANGE));
        }
        Art::ChartWaterfall => {
            for (i, (a, b, c)) in [(0.3, 0.92, BLUE), (0.15, 0.3, GREEN), (0.15, 0.5, RED), (0.5, 0.92, BLUE)].iter().enumerate() {
                let x = 0.08 + i as f32 * 0.22;
                p.rect_filled(ur(r, x, *a, x + 0.16, *b), CornerRadius::ZERO, *c);
            }
        }
        Art::ChartScatter => {
            for (x, y) in [(0.2, 0.75), (0.35, 0.5), (0.5, 0.6), (0.62, 0.3), (0.8, 0.22), (0.75, 0.5)] {
                p.circle_filled(u(r, x, y), r.width() * 0.06, BLUE);
            }
            path(p, vec![u(r, 0.08, 0.08), u(r, 0.08, 0.92), u(r, 0.92, 0.92)], s);
        }
        Art::Maps => {
            p.circle_filled(u(r, 0.5, 0.5), r.width() * 0.44, Color32::from_rgb(0x9C, 0xC9, 0xF0));
            poly(p, vec![u(r, 0.25, 0.3), u(r, 0.5, 0.2), u(r, 0.55, 0.45), u(r, 0.35, 0.6)], DARK_GREEN, Stroke::NONE);
            poly(p, vec![u(r, 0.6, 0.55), u(r, 0.8, 0.5), u(r, 0.72, 0.8)], DARK_GREEN, Stroke::NONE);
            p.circle_stroke(u(r, 0.5, 0.5), r.width() * 0.44, s);
        }
        Art::PivotChart => {
            sheet(p, ur(r, 0.05, 0.05, 0.7, 0.75), ink, Color32::from_gray(225), 3, 2);
            bars(p, ur(r, 0.45, 0.4, 0.98, 0.98), &[0.5, 1.0, 0.7], BLUE);
        }
        Art::SparkLine => {
            path(p, vec![u(r, 0.05, 0.7), u(r, 0.3, 0.35), u(r, 0.5, 0.6), u(r, 0.7, 0.3), u(r, 0.95, 0.45)], Stroke::new(w * 1.3, BLUE))
        }
        Art::SparkColumn => bars(p, ur(r, 0.05, 0.2, 0.95, 0.85), &[0.4, 0.8, 0.55, 1.0, 0.6], BLUE),
        Art::SparkWinLoss => {
            for (i, up) in [true, false, true, true, false].iter().enumerate() {
                let x = 0.07 + i as f32 * 0.18;
                let (y0, y1) = if *up { (0.2, 0.48) } else { (0.52, 0.8) };
                p.rect_filled(ur(r, x, y0, x + 0.12, y1), CornerRadius::ZERO, if *up { BLUE } else { RED });
            }
        }
        Art::Slicer => {
            p.rect_stroke(ur(r, 0.08, 0.05, 0.92, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            for (i, c) in [BLUE, BLUE, PAPER].iter().enumerate() {
                let y = 0.22 + i as f32 * 0.24;
                p.rect_filled(ur(r, 0.2, y, 0.8, y + 0.16), CornerRadius::ZERO, *c);
                p.rect_stroke(ur(r, 0.2, y, 0.8, y + 0.16), CornerRadius::ZERO, Stroke::new(w * 0.7, ink), StrokeKind::Inside);
            }
        }
        Art::Timeline => {
            p.rect_stroke(ur(r, 0.05, 0.15, 0.95, 0.85), CornerRadius::same(1), s, StrokeKind::Inside);
            p.rect_filled(ur(r, 0.15, 0.45, 0.6, 0.65), CornerRadius::ZERO, BLUE);
            p.hline(u(r, 0.15, 0.0).x..=u(r, 0.85, 0.0).x, u(r, 0.0, 0.32).y, Stroke::new(w * 0.7, GRAY));
        }
        Art::Link => {
            let sw = Stroke::new(w * 1.3, ink);
            p.rect_stroke(ur(r, 0.05, 0.3, 0.6, 0.7), CornerRadius::same(5), sw, StrokeKind::Middle);
            p.rect_stroke(ur(r, 0.4, 0.3, 0.95, 0.7), CornerRadius::same(5), Stroke::new(w * 1.3, BLUE), StrokeKind::Middle);
        }
        Art::Comment | Art::NewComment | Art::ShowComments => {
            bubble(p, ur(r, 0.05, 0.1, 0.95, 0.95), ink, PAPER);
            if art == Art::NewComment {
                line(p, u(r, 0.2, 0.0), u(r, 0.2, 0.36), Stroke::new(w * 1.3, GREEN));
                line(p, u(r, 0.02, 0.18), u(r, 0.38, 0.18), Stroke::new(w * 1.3, GREEN));
            }
            if art == Art::Comment {
                line(p, u(r, 0.5, 0.0), u(r, 0.5, 0.3), Stroke::new(w * 1.3, GREEN));
            }
        }
        Art::TextBox => {
            p.rect_stroke(ur(r, 0.08, 0.08, 0.92, 0.92), CornerRadius::ZERO, Stroke::new(w, BLUE), StrokeKind::Inside);
            for (x, y) in [(0.08, 0.08), (0.92, 0.08), (0.08, 0.92), (0.92, 0.92)] {
                p.rect_filled(Rect::from_center_size(u(r, x, y), vec2(w * 2.5, w * 2.5)), CornerRadius::ZERO, BLUE);
            }
            glyph(p, ur(r, 0.08, 0.08, 0.92, 0.92), "A", 0.65, ink);
        }
        Art::HeaderFooter => {
            page(p, ur(r, 0.12, 0.05, 0.88, 0.95), ink);
            p.rect_filled(ur(r, 0.22, 0.14, 0.72, 0.26), CornerRadius::ZERO, ORANGE);
            p.rect_filled(ur(r, 0.22, 0.78, 0.78, 0.88), CornerRadius::ZERO, ORANGE);
        }
        Art::WordArt => {
            glyph(p, ur(r, 0.0, 0.0, 1.0, 1.0), "A", 0.9, BLUE);
        }
        Art::SignatureLine => {
            path(p, vec![u(r, 0.1, 0.65), u(r, 0.3, 0.3), u(r, 0.45, 0.6), u(r, 0.65, 0.35), u(r, 0.9, 0.55)], Stroke::new(w, BLUE));
            line(p, u(r, 0.05, 0.85), u(r, 0.95, 0.85), s);
        }
        Art::Object => {
            p.rect_stroke(ur(r, 0.08, 0.15, 0.92, 0.85), CornerRadius::ZERO, s, StrokeKind::Inside);
            p.rect_filled(ur(r, 0.25, 0.35, 0.75, 0.65), CornerRadius::ZERO, BLUE);
        }
        Art::Equation => glyph(p, r, "π", 0.9, ink),
        Art::Symbol => glyph(p, r, "Ω", 0.85, ink),
        // ------------------------------------------------------------------ Draw
        Art::Undo | Art::Redo => {
            // An open arc with an arrowhead at its start.
            let dir = if art == Art::Undo { 1.0 } else { -1.0 };
            let c = u(r, 0.5, 0.58);
            let rad = r.width() * 0.32;
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0 * 1.3);
                    pos2(c.x + dir * rad * a.cos(), c.y + rad * a.sin())
                })
                .collect();
            path(p, pts, s);
            let tip = pos2(c.x - dir * rad, c.y);
            poly(p, vec![pos2(tip.x - 3.5, tip.y - 2.0), pos2(tip.x + 3.5, tip.y - 2.0), pos2(tip.x, tip.y + 3.5)], ink, Stroke::NONE);
        }
        Art::Select => {
            let pts =
                vec![u(r, 0.25, 0.08), u(r, 0.25, 0.85), u(r, 0.42, 0.68), u(r, 0.55, 0.95), u(r, 0.66, 0.9), u(r, 0.54, 0.62), u(r, 0.78, 0.62)];
            let mut outline = pts.clone();
            outline.push(pts[0]);
            p.add(Shape::convex_polygon(vec![pts[0], pts[1], pts[2], pts[6]], PAPER, Stroke::NONE));
            path(p, outline, Stroke::new(w, ink));
        }
        Art::Lasso => {
            let c = u(r, 0.5, 0.42);
            let n = 18;
            for i in 0..n {
                if i % 2 == 0 {
                    let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
                    let a1 = (i as f32 + 1.0) / n as f32 * std::f32::consts::TAU;
                    let rr = r.width() * 0.36;
                    line(p, pos2(c.x + rr * a0.cos(), c.y + rr * 0.85 * a0.sin()), pos2(c.x + rr * a1.cos(), c.y + rr * 0.85 * a1.sin()), s);
                }
            }
            p.circle_stroke(u(r, 0.45, 0.84), r.width() * 0.08, s);
        }
        Art::Eraser => {
            p.rect_filled(ur(r, 0.3, 0.0, 0.7, 0.62), CornerRadius::same(2), Color32::from_gray(200));
            p.rect_filled(ur(r, 0.3, 0.62, 0.7, 0.95), CornerRadius::same(2), Color32::from_rgb(0xD8, 0x9E, 0xE0));
            p.rect_stroke(ur(r, 0.3, 0.0, 0.7, 0.95), CornerRadius::same(2), s, StrokeKind::Inside);
        }
        Art::Pen(c) => pen(p, r, c, false),
        Art::Highlighter(c) => pen(p, r, c, true),
        Art::Pencil(c) => {
            pen(p, r, c, false);
            p.rect_filled(ur(r, 0.3, 0.0, 0.7, 0.6), CornerRadius::same(2), c);
            p.rect_stroke(ur(r, 0.3, 0.0, 0.7, 0.6), CornerRadius::same(2), Stroke::new(w, Color32::from_gray(40)), StrokeKind::Inside);
        }
        Art::GalaxyPen => {
            pen(p, r, GREEN, false);
            for (x, y) in [(0.12, 0.62), (0.2, 0.85), (0.86, 0.72)] {
                glyph(p, Rect::from_center_size(u(r, x, y), vec2(r.width() * 0.25, r.height() * 0.25)), "✦", 0.9, ink);
            }
        }
        Art::AddPen => {
            line(p, u(r, 0.5, 0.1), u(r, 0.5, 0.9), Stroke::new(w * 1.2, GREEN));
            line(p, u(r, 0.1, 0.5), u(r, 0.9, 0.5), Stroke::new(w * 1.2, GREEN));
        }
        Art::InkToShape => {
            p.rect_stroke(ur(r, 0.3, 0.35, 0.95, 0.95), CornerRadius::same(2), Stroke::new(w, BLUE), StrokeKind::Inside);
            path(p, vec![u(r, 0.05, 0.25), u(r, 0.7, 0.25)], s);
            path(p, vec![u(r, 0.1, 0.1), u(r, 0.1, 0.25)], s);
            path(p, vec![u(r, 0.6, 0.1), u(r, 0.6, 0.25)], s);
        }
        Art::InkToMath => {
            glyph(p, ur(r, 0.45, 0.0, 1.0, 0.45), "π", 1.0, ink);
            glyph(p, ur(r, 0.4, 0.45, 1.0, 1.0), "π", 1.0, BLUE);
            path(p, vec![u(r, 0.2, 0.2), u(r, 0.1, 0.5), u(r, 0.2, 0.8), u(r, 0.4, 0.8)], Stroke::new(w, BLUE));
        }
        Art::InkReplay => {
            p.circle_stroke(u(r, 0.5, 0.5), r.width() * 0.4, s);
            poly(p, vec![u(r, 0.42, 0.32), u(r, 0.42, 0.68), u(r, 0.7, 0.5)], GREEN, Stroke::NONE);
        }
        Art::Help => {
            p.circle_stroke(u(r, 0.5, 0.5), r.width() * 0.44, Stroke::new(w, BLUE));
            glyph(p, r, "?", 0.62, BLUE);
        }
        // ------------------------------------------------------------------ Formulas
        Art::InsertFunction => glyph(p, r, "fx", 0.75, ink),
        Art::AutoSum => glyph(p, r, "Σ", 0.9, ink),
        Art::RecentlyUsed => {
            p.rect_stroke(ur(r, 0.1, 0.08, 0.9, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            glyph(p, ur(r, 0.1, 0.1, 0.9, 0.9), "★", 0.7, BLUE);
        }
        Art::Financial => {
            for i in 0..3 {
                let y = 0.18 + i as f32 * 0.24;
                p.rect_filled(ur(r, 0.2, y, 0.8, y + 0.22), CornerRadius::same(6), GREEN);
                p.rect_stroke(ur(r, 0.2, y, 0.8, y + 0.22), CornerRadius::same(6), Stroke::new(w * 0.8, DARK_GREEN), StrokeKind::Inside);
            }
        }
        Art::Logical => {
            p.rect_stroke(ur(r, 0.1, 0.08, 0.9, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            glyph(p, ur(r, 0.1, 0.1, 0.9, 0.9), "?", 0.8, PURPLE);
        }
        Art::Text => {
            p.rect_stroke(ur(r, 0.1, 0.08, 0.9, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            glyph(p, ur(r, 0.1, 0.1, 0.9, 0.9), "A", 0.8, BLUE);
        }
        Art::DateTime => {
            p.rect_stroke(ur(r, 0.1, 0.08, 0.9, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            p.circle_stroke(u(r, 0.5, 0.52), r.width() * 0.26, Stroke::new(w, RED));
            path(p, vec![u(r, 0.5, 0.35), u(r, 0.5, 0.52), u(r, 0.62, 0.6)], Stroke::new(w, RED));
        }
        Art::Lookup => {
            p.rect_stroke(ur(r, 0.1, 0.08, 0.9, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            magnifier(p, u(r, 0.45, 0.45), r.width() * 0.17, BLUE);
        }
        Art::MathTrig => {
            p.rect_stroke(ur(r, 0.1, 0.08, 0.9, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            glyph(p, ur(r, 0.1, 0.1, 0.9, 0.9), "θ", 0.8, GREEN);
        }
        Art::MoreFunctions => {
            p.rect_stroke(ur(r, 0.1, 0.08, 0.9, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            for x in [0.3, 0.5, 0.7] {
                p.circle_filled(u(r, x, 0.5), r.width() * 0.05, RED);
            }
        }
        Art::Python => {
            p.rect_filled(ur(r, 0.1, 0.1, 0.62, 0.55), CornerRadius::same(4), BLUE);
            p.rect_filled(ur(r, 0.38, 0.45, 0.9, 0.9), CornerRadius::same(4), YELLOW);
        }
        Art::Reset => {
            p.circle_stroke(u(r, 0.5, 0.55), r.width() * 0.32, s);
            poly(p, vec![u(r, 0.1, 0.35), u(r, 0.3, 0.35), u(r, 0.2, 0.55)], ink, Stroke::NONE);
        }
        Art::Editor => {
            line(p, u(r, 0.15, 0.85), u(r, 0.8, 0.2), Stroke::new(w * 2.0, ink));
            line(p, u(r, 0.08, 0.95), u(r, 0.15, 0.85), Stroke::new(w, ink));
        }
        Art::Initialization => {
            page(p, ur(r, 0.15, 0.05, 0.85, 0.95), ink);
            glyph(p, ur(r, 0.15, 0.3, 0.85, 0.95), "⚙", 0.7, BLUE);
        }
        Art::NameManager => {
            poly(p, vec![u(r, 0.1, 0.45), u(r, 0.45, 0.1), u(r, 0.9, 0.1), u(r, 0.9, 0.8), u(r, 0.45, 0.8)], PAPER, s);
            sheet(p, ur(r, 0.5, 0.5, 0.98, 0.98), ink, BLUE, 2, 2);
        }
        Art::DefineName => {
            poly(p, vec![u(r, 0.05, 0.5), u(r, 0.4, 0.15), u(r, 0.95, 0.15), u(r, 0.95, 0.85), u(r, 0.4, 0.85)], PAPER, s);
            p.circle_filled(u(r, 0.38, 0.5), r.width() * 0.07, ink);
        }
        Art::UseInFormula => glyph(p, r, "fx", 0.7, GRAY),
        Art::CreateFromSelection => {
            sheet(p, ur(r, 0.05, 0.1, 0.95, 0.9), ink, Color32::from_gray(225), 3, 3);
            p.rect_filled(ur(r, 0.05, 0.1, 0.35, 0.9), CornerRadius::ZERO, Color32::from_rgba_unmultiplied(0x2B, 0x7C, 0xD3, 80));
        }
        Art::TracePrecedents | Art::TraceDependents => {
            p.rect_stroke(ur(r, 0.05, 0.05, 0.4, 0.4), CornerRadius::ZERO, s, StrokeKind::Inside);
            p.rect_stroke(ur(r, 0.55, 0.55, 0.95, 0.95), CornerRadius::ZERO, s, StrokeKind::Inside);
            let (a, b) = if art == Art::TracePrecedents { (u(r, 0.25, 0.25), u(r, 0.7, 0.7)) } else { (u(r, 0.75, 0.75), u(r, 0.3, 0.3)) };
            line(p, a, b, Stroke::new(w, BLUE));
            p.circle_filled(a, w * 1.6, BLUE);
            poly(p, vec![pos2(b.x - 4.0, b.y), pos2(b.x, b.y - 4.0), pos2(b.x + 1.0, b.y + 1.0)], BLUE, Stroke::NONE);
        }
        Art::RemoveArrows => {
            line(p, u(r, 0.1, 0.5), u(r, 0.7, 0.5), Stroke::new(w, BLUE));
            line(p, u(r, 0.6, 0.6), u(r, 0.95, 0.95), Stroke::new(w * 1.3, RED));
            line(p, u(r, 0.95, 0.6), u(r, 0.6, 0.95), Stroke::new(w * 1.3, RED));
        }
        Art::ShowFormulas => {
            p.rect_stroke(ur(r, 0.05, 0.1, 0.95, 0.9), CornerRadius::same(1), s, StrokeKind::Inside);
            glyph(p, ur(r, 0.05, 0.1, 0.95, 0.9), "fx", 0.65, ink);
        }
        Art::ErrorChecking => {
            poly(p, vec![u(r, 0.5, 0.08), u(r, 0.95, 0.9), u(r, 0.05, 0.9)], YELLOW, Stroke::new(w, ORANGE));
            glyph(p, ur(r, 0.2, 0.3, 0.8, 0.95), "!", 0.8, ink);
        }
        Art::EvaluateFormula => {
            p.circle_stroke(u(r, 0.5, 0.5), r.width() * 0.42, s);
            glyph(p, r, "fx", 0.5, ink);
        }
        Art::WatchWindow => {
            p.rect_stroke(ur(r, 0.05, 0.1, 0.95, 0.85), CornerRadius::same(1), s, StrokeKind::Inside);
            p.rect_filled(ur(r, 0.05, 0.1, 0.95, 0.25), CornerRadius::ZERO, BLUE);
            p.circle_stroke(u(r, 0.38, 0.62), r.width() * 0.11, Stroke::new(w, BLUE));
            p.circle_stroke(u(r, 0.66, 0.62), r.width() * 0.11, Stroke::new(w, BLUE));
        }
        Art::CalcOptions | Art::CalcNow | Art::CalcSheet => {
            let body = ur(r, 0.18, 0.05, 0.82, 0.95);
            p.rect_filled(body, CornerRadius::same(2), PAPER);
            p.rect_stroke(body, CornerRadius::same(2), s, StrokeKind::Inside);
            p.rect_filled(ur(r, 0.28, 0.14, 0.72, 0.32), CornerRadius::ZERO, if art == Art::CalcSheet { GREEN } else { BLUE });
            for row in 0..3 {
                for col in 0..3 {
                    let x = 0.3 + col as f32 * 0.15;
                    let y = 0.45 + row as f32 * 0.15;
                    p.rect_filled(ur(r, x, y, x + 0.1, y + 0.1), CornerRadius::ZERO, ink);
                }
            }
        }
        // ------------------------------------------------------------------ Data
        Art::GetData => {
            let c = ur(r, 0.15, 0.05, 0.85, 0.95);
            p.rect_filled(c, CornerRadius::same(6), PAPER);
            p.rect_stroke(c, CornerRadius::same(6), s, StrokeKind::Inside);
            for y in [0.3, 0.55] {
                p.hline(c.x_range(), u(r, 0.0, y).y, s);
            }
        }
        Art::FromCsv => {
            page(p, ur(r, 0.15, 0.05, 0.85, 0.95), ink);
            lines(p, ur(r, 0.25, 0.35, 0.75, 0.85), 3, GRAY);
        }
        Art::FromWeb => {
            page(p, ur(r, 0.15, 0.05, 0.85, 0.95), ink);
            p.circle_filled(u(r, 0.5, 0.62), r.width() * 0.2, BLUE);
            p.circle_stroke(u(r, 0.5, 0.62), r.width() * 0.2, Stroke::new(w * 0.8, PAPER));
        }
        Art::FromTable => sheet(p, ur(r, 0.08, 0.12, 0.92, 0.88), ink, BLUE, 3, 3),
        Art::FromPicture => {
            sheet(p, ur(r, 0.05, 0.05, 0.75, 0.75), ink, BLUE, 2, 2);
            p.rect_filled(ur(r, 0.45, 0.5, 0.98, 0.98), CornerRadius::ZERO, GREEN);
        }
        Art::RecentSources => {
            page(p, ur(r, 0.1, 0.05, 0.75, 0.95), ink);
            p.circle_stroke(u(r, 0.68, 0.68), r.width() * 0.22, Stroke::new(w, BLUE));
        }
        Art::ExistingConnections => {
            page(p, ur(r, 0.15, 0.05, 0.85, 0.95), ink);
            sheet(p, ur(r, 0.3, 0.45, 0.7, 0.85), ink, ORANGE, 2, 2);
        }
        Art::RefreshAll => {
            page(p, ur(r, 0.1, 0.05, 0.7, 0.85), ink);
            p.circle_stroke(u(r, 0.68, 0.7), r.width() * 0.22, Stroke::new(w * 1.3, GREEN));
        }
        Art::Queries | Art::Properties | Art::WorkbookLinks => {
            sheet(p, ur(r, 0.08, 0.12, 0.92, 0.88), ink, if art == Art::Queries { BLUE } else { GRAY }, 3, 1);
        }
        Art::Stocks => {
            poly(p, vec![u(r, 0.5, 0.05), u(r, 0.95, 0.3), u(r, 0.05, 0.3)], PAPER, s);
            for x in [0.2, 0.4, 0.6, 0.8] {
                line(p, u(r, x, 0.35), u(r, x, 0.82), s);
            }
            line(p, u(r, 0.05, 0.88), u(r, 0.95, 0.88), Stroke::new(w * 1.4, ink));
        }
        Art::Currencies => {
            p.rect_stroke(ur(r, 0.05, 0.25, 0.95, 0.85), CornerRadius::same(2), s, StrokeKind::Inside);
            p.circle_stroke(u(r, 0.5, 0.55), r.width() * 0.17, s);
        }
        Art::SortAz | Art::SortZa => {
            let (a, b) = if art == Art::SortAz { ("A", "Z") } else { ("Z", "A") };
            glyph(p, ur(r, 0.0, 0.0, 0.55, 0.5), a, 0.95, BLUE);
            glyph(p, ur(r, 0.0, 0.5, 0.55, 1.0), b, 0.95, ink);
            line(p, u(r, 0.75, 0.1), u(r, 0.75, 0.9), s);
            poly(p, vec![u(r, 0.62, 0.7), u(r, 0.88, 0.7), u(r, 0.75, 0.92)], ink, Stroke::NONE);
        }
        Art::Sort => {
            p.rect_stroke(ur(r, 0.08, 0.05, 0.92, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            glyph(p, ur(r, 0.08, 0.05, 0.92, 0.5), "A Z", 0.6, BLUE);
            glyph(p, ur(r, 0.08, 0.5, 0.92, 0.95), "Z A", 0.6, ink);
        }
        Art::Filter => funnel(p, ur(r, 0.08, 0.05, 0.92, 0.95), ink, PAPER),
        Art::ClearFilter => {
            funnel(p, ur(r, 0.05, 0.0, 0.75, 0.8), ink, PAPER);
            line(p, u(r, 0.6, 0.6), u(r, 0.95, 0.95), Stroke::new(w * 1.3, RED));
            line(p, u(r, 0.95, 0.6), u(r, 0.6, 0.95), Stroke::new(w * 1.3, RED));
        }
        Art::Reapply => {
            funnel(p, ur(r, 0.05, 0.0, 0.75, 0.8), ink, PAPER);
            p.circle_stroke(u(r, 0.78, 0.78), r.width() * 0.17, Stroke::new(w * 1.2, GREEN));
        }
        Art::Advanced => {
            funnel(p, ur(r, 0.05, 0.0, 0.75, 0.8), ink, PAPER);
            sheet(p, ur(r, 0.55, 0.55, 0.98, 0.98), ink, BLUE, 2, 2);
        }
        Art::TextToColumns => {
            page(p, ur(r, 0.08, 0.05, 0.62, 0.95), ink);
            lines(p, ur(r, 0.16, 0.3, 0.54, 0.85), 3, GRAY);
            for x in [0.72, 0.86] {
                line(p, u(r, x, 0.1), u(r, x, 0.9), Stroke::new(w * 1.2, BLUE));
            }
        }
        Art::FlashFill => {
            sheet(p, ur(r, 0.05, 0.05, 0.95, 0.95), ink, Color32::from_gray(225), 3, 2);
            poly(p, vec![u(r, 0.55, 0.25), u(r, 0.75, 0.25), u(r, 0.6, 0.5), u(r, 0.8, 0.5), u(r, 0.5, 0.95)], YELLOW, Stroke::NONE);
        }
        Art::RemoveDuplicates => {
            sheet(p, ur(r, 0.05, 0.05, 0.8, 0.8), ink, Color32::from_gray(225), 3, 2);
            line(p, u(r, 0.6, 0.6), u(r, 0.95, 0.95), Stroke::new(w * 1.4, RED));
            line(p, u(r, 0.95, 0.6), u(r, 0.6, 0.95), Stroke::new(w * 1.4, RED));
        }
        Art::DataValidation => {
            sheet(p, ur(r, 0.05, 0.05, 0.8, 0.8), ink, Color32::from_gray(225), 3, 2);
            path(p, vec![u(r, 0.55, 0.78), u(r, 0.7, 0.92), u(r, 0.98, 0.55)], Stroke::new(w * 1.6, GREEN));
        }
        Art::Consolidate => {
            for (x, y) in [(0.05, 0.05), (0.05, 0.55)] {
                p.rect_filled(ur(r, x, y, x + 0.35, y + 0.35), CornerRadius::ZERO, BLUE);
            }
            p.rect_filled(ur(r, 0.6, 0.3, 0.95, 0.7), CornerRadius::ZERO, GREEN);
            line(p, u(r, 0.42, 0.22), u(r, 0.6, 0.45), s);
            line(p, u(r, 0.42, 0.72), u(r, 0.6, 0.55), s);
        }
        Art::DataModel => {
            for (x, y) in [(0.05, 0.05), (0.55, 0.05), (0.3, 0.55)] {
                sheet(p, ur(r, x, y, x + 0.4, y + 0.4), ink, GREEN, 2, 2);
            }
        }
        Art::AnalyzeData => {
            bars(p, ur(r, 0.05, 0.3, 0.7, 0.95), &[0.5, 0.8, 0.6], BLUE);
            glyph(p, ur(r, 0.55, 0.0, 1.0, 0.45), "✦", 0.9, PURPLE);
        }
        Art::WhatIf => {
            sheet(p, ur(r, 0.05, 0.05, 0.85, 0.9), ink, Color32::from_gray(225), 3, 3);
            glyph(p, ur(r, 0.55, 0.4, 1.0, 1.0), "?", 0.9, BLUE);
        }
        Art::ForecastSheet => {
            p.rect_stroke(ur(r, 0.05, 0.08, 0.95, 0.92), CornerRadius::same(1), s, StrokeKind::Inside);
            path(p, vec![u(r, 0.12, 0.75), u(r, 0.35, 0.55), u(r, 0.55, 0.62)], Stroke::new(w * 1.3, BLUE));
            path(p, vec![u(r, 0.55, 0.62), u(r, 0.88, 0.3)], Stroke::new(w * 1.3, ORANGE));
        }
        Art::Group | Art::Ungroup => {
            sheet(p, ur(r, 0.2, 0.05, 0.95, 0.95), ink, Color32::from_gray(225), 3, 2);
            path(p, vec![u(r, 0.12, 0.15), u(r, 0.05, 0.15), u(r, 0.05, 0.85), u(r, 0.12, 0.85)], Stroke::new(w, BLUE));
            if art == Art::Ungroup {
                line(p, u(r, 0.55, 0.3), u(r, 0.85, 0.7), Stroke::new(w * 1.3, RED));
            }
        }
        Art::Subtotal => {
            sheet(p, ur(r, 0.05, 0.05, 0.95, 0.95), ink, Color32::from_gray(225), 4, 2);
            p.rect_filled(ur(r, 0.05, 0.58, 0.95, 0.76), CornerRadius::ZERO, Color32::from_rgba_unmultiplied(0x2B, 0x7C, 0xD3, 90));
        }
        Art::ShowDetail | Art::HideDetail => {
            lines(p, ur(r, 0.4, 0.15, 0.95, 0.85), 3, ink);
            let c = u(r, 0.18, 0.5);
            line(p, pos2(c.x - 4.0, c.y), pos2(c.x + 4.0, c.y), s);
            if art == Art::ShowDetail {
                line(p, pos2(c.x, c.y - 4.0), pos2(c.x, c.y + 4.0), s);
            }
        }
        // ------------------------------------------------------------------ Review
        Art::Spelling => {
            glyph(p, ur(r, 0.0, 0.0, 1.0, 0.55), "abc", 0.75, ink);
            path(p, vec![u(r, 0.3, 0.72), u(r, 0.45, 0.9), u(r, 0.75, 0.6)], Stroke::new(w * 1.4, GREEN));
        }
        Art::Thesaurus => {
            poly(p, vec![u(r, 0.05, 0.15), u(r, 0.5, 0.25), u(r, 0.5, 0.95), u(r, 0.05, 0.85)], PAPER, s);
            poly(p, vec![u(r, 0.5, 0.25), u(r, 0.95, 0.15), u(r, 0.95, 0.85), u(r, 0.5, 0.95)], PAPER, s);
            lines(p, ur(r, 0.12, 0.35, 0.42, 0.8), 3, RED);
        }
        Art::WorkbookStats => {
            sheet(p, ur(r, 0.05, 0.05, 0.85, 0.85), ink, Color32::from_gray(225), 3, 2);
            glyph(p, ur(r, 0.45, 0.5, 1.0, 1.0), "123", 0.6, BLUE);
        }
        Art::CheckPerformance => {
            sheet(p, ur(r, 0.05, 0.05, 0.8, 0.8), ink, Color32::from_gray(225), 3, 2);
            p.circle_filled(u(r, 0.75, 0.75), r.width() * 0.22, PAPER);
            p.circle_stroke(u(r, 0.75, 0.75), r.width() * 0.22, Stroke::new(w, BLUE));
            path(p, vec![u(r, 0.75, 0.62), u(r, 0.75, 0.75), u(r, 0.85, 0.8)], Stroke::new(w, BLUE));
        }
        Art::Accessibility => {
            page(p, ur(r, 0.15, 0.05, 0.85, 0.95), ink);
            p.circle_filled(u(r, 0.5, 0.4), r.width() * 0.06, BLUE);
            path(p, vec![u(r, 0.32, 0.5), u(r, 0.68, 0.5)], Stroke::new(w, BLUE));
            path(p, vec![u(r, 0.4, 0.82), u(r, 0.5, 0.55), u(r, 0.6, 0.82)], Stroke::new(w, BLUE));
        }
        Art::Translate => {
            glyph(p, ur(r, 0.0, 0.4, 0.55, 1.0), "a", 1.0, BLUE);
            glyph(p, ur(r, 0.45, 0.0, 1.0, 0.6), "あ", 0.9, GREEN);
        }
        Art::ShowChanges => {
            sheet(p, ur(r, 0.05, 0.05, 0.8, 0.8), ink, Color32::from_gray(225), 3, 2);
            p.circle_stroke(u(r, 0.75, 0.75), r.width() * 0.2, Stroke::new(w, ink));
        }
        Art::DeleteComment => {
            bubble(p, ur(r, 0.05, 0.1, 0.85, 0.95), ink, PAPER);
            line(p, u(r, 0.05, 0.05), u(r, 0.35, 0.35), Stroke::new(w * 1.2, RED));
            line(p, u(r, 0.35, 0.05), u(r, 0.05, 0.35), Stroke::new(w * 1.2, RED));
        }
        Art::PreviousComment | Art::NextComment => {
            bubble(p, ur(r, 0.15, 0.1, 0.95, 0.95), ink, PAPER);
            let (a, b) = if art == Art::PreviousComment { (0.6, 0.0) } else { (0.0, 0.6) };
            path(p, vec![u(r, a + 0.1, 0.4), u(r, b + 0.1, 0.4)], Stroke::new(w * 1.3, ink));
        }
        Art::Notes => {
            poly(p, vec![u(r, 0.1, 0.08), u(r, 0.9, 0.08), u(r, 0.9, 0.68), u(r, 0.68, 0.92), u(r, 0.1, 0.92)], PAPER, s);
            path(p, vec![u(r, 0.68, 0.92), u(r, 0.68, 0.68), u(r, 0.9, 0.68)], s);
        }
        Art::ProtectSheet => {
            sheet(p, ur(r, 0.05, 0.05, 0.8, 0.8), ink, Color32::from_gray(225), 3, 2);
            padlock(p, r);
        }
        Art::ProtectWorkbook => {
            sheet(p, ur(r, 0.15, 0.0, 0.85, 0.65), ink, Color32::from_gray(225), 2, 2);
            sheet(p, ur(r, 0.05, 0.12, 0.75, 0.8), ink, Color32::from_gray(225), 3, 2);
            padlock(p, r);
        }
        Art::AllowEditRanges => {
            sheet(p, ur(r, 0.05, 0.05, 0.8, 0.8), ink, Color32::from_gray(225), 3, 2);
            line(p, u(r, 0.45, 0.95), u(r, 0.95, 0.4), Stroke::new(w * 2.0, BLUE));
        }
        Art::UnshareWorkbook => {
            sheet(p, ur(r, 0.05, 0.05, 0.8, 0.8), ink, Color32::from_gray(225), 3, 2);
            p.circle_filled(u(r, 0.75, 0.6), r.width() * 0.09, GRAY);
            p.circle_filled(u(r, 0.75, 0.9), r.width() * 0.14, GRAY);
        }
        Art::HideInk => {
            path(p, vec![u(r, 0.1, 0.1), u(r, 0.9, 0.1), u(r, 0.9, 0.9)], s);
            path(p, vec![u(r, 0.1, 0.75), u(r, 0.35, 0.5), u(r, 0.55, 0.7), u(r, 0.9, 0.4)], Stroke::new(w * 1.3, RED));
            line(p, u(r, 0.1, 0.1), u(r, 0.9, 0.9), s);
        }
        // ------------------------------------------------------------------ View
        Art::Keep => {
            p.rect_stroke(ur(r, 0.1, 0.1, 0.9, 0.9), CornerRadius::same(1), s, StrokeKind::Inside);
            p.rect_filled(ur(r, 0.3, 0.1, 0.7, 0.4), CornerRadius::ZERO, ink);
        }
        Art::Exit => {
            p.circle_stroke(u(r, 0.5, 0.5), r.width() * 0.25, s);
            line(p, u(r, 0.1, 0.5), u(r, 0.9, 0.5), s);
        }
        Art::NewView => {
            p.circle_stroke(u(r, 0.5, 0.5), r.width() * 0.25, s);
            line(p, u(r, 0.8, 0.1), u(r, 0.8, 0.4), Stroke::new(w, GREEN));
            line(p, u(r, 0.65, 0.25), u(r, 0.95, 0.25), Stroke::new(w, GREEN));
        }
        Art::ViewOptions => lines(p, ur(r, 0.1, 0.15, 0.9, 0.85), 3, ink),
        Art::Normal => sheet(p, ur(r, 0.08, 0.08, 0.92, 0.92), ink, Color32::from_gray(225), 3, 3),
        Art::PageBreakPreview => {
            sheet(p, ur(r, 0.05, 0.08, 0.95, 0.92), ink, Color32::from_gray(225), 3, 3);
            line(p, u(r, 0.5, 0.08), u(r, 0.5, 0.92), Stroke::new(w * 1.4, BLUE));
            line(p, u(r, 0.05, 0.6), u(r, 0.95, 0.6), Stroke::new(w * 1.4, BLUE));
        }
        Art::PageLayoutView => {
            page(p, ur(r, 0.15, 0.05, 0.85, 0.95), ink);
            sheet(p, ur(r, 0.25, 0.3, 0.75, 0.8), ink, Color32::from_gray(225), 2, 2);
        }
        Art::CustomViews => {
            page(p, ur(r, 0.2, 0.05, 0.9, 0.85), ink);
            page(p, ur(r, 0.1, 0.15, 0.8, 0.95), ink);
            lines(p, ur(r, 0.2, 0.4, 0.7, 0.85), 3, GRAY);
        }
        Art::Navigation => {
            p.rect_stroke(ur(r, 0.05, 0.1, 0.95, 0.9), CornerRadius::same(1), s, StrokeKind::Inside);
            p.rect_filled(ur(r, 0.05, 0.1, 0.4, 0.9), CornerRadius::ZERO, Color32::from_rgb(0x9C, 0xC9, 0xF0));
        }
        Art::FocusCell => {
            p.rect_filled(ur(r, 0.0, 0.38, 1.0, 0.62), CornerRadius::ZERO, Color32::from_rgb(0x9C, 0xC9, 0xF0));
            p.rect_filled(ur(r, 0.38, 0.0, 0.62, 1.0), CornerRadius::ZERO, Color32::from_rgb(0x9C, 0xC9, 0xF0));
            p.rect_stroke(ur(r, 0.38, 0.38, 0.62, 0.62), CornerRadius::ZERO, Stroke::new(w, BLUE), StrokeKind::Inside);
        }
        Art::Zoom => magnifier(p, u(r, 0.4, 0.4), r.width() * 0.28, ink),
        Art::Zoom100 => {
            page(p, ur(r, 0.1, 0.05, 0.9, 0.95), ink);
            p.rect_filled(ur(r, 0.2, 0.6, 0.98, 0.9), CornerRadius::ZERO, BLUE);
            glyph(p, ur(r, 0.2, 0.6, 0.98, 0.9), "100", 0.8, PAPER);
        }
        Art::ZoomSelection => {
            sheet(p, ur(r, 0.05, 0.05, 0.8, 0.8), ink, Color32::from_gray(225), 3, 2);
            magnifier(p, u(r, 0.65, 0.62), r.width() * 0.16, BLUE);
        }
        Art::NewWindow => {
            p.rect_stroke(ur(r, 0.1, 0.2, 0.95, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            line(p, u(r, 0.18, 0.0), u(r, 0.18, 0.4), Stroke::new(w * 1.3, GREEN));
            line(p, u(r, 0.0, 0.2), u(r, 0.36, 0.2), Stroke::new(w * 1.3, GREEN));
        }
        Art::ArrangeAll => {
            for y in [0.1, 0.55] {
                p.rect_stroke(ur(r, 0.1, y, 0.9, y + 0.35), CornerRadius::same(1), s, StrokeKind::Inside);
                p.rect_filled(ur(r, 0.1, y, 0.9, y + 0.09), CornerRadius::ZERO, ink);
            }
        }
        Art::FreezePanes => {
            sheet(p, ur(r, 0.05, 0.08, 0.95, 0.95), ink, Color32::from_gray(225), 3, 3);
            p.rect_filled(ur(r, 0.05, 0.08, 0.35, 0.95), CornerRadius::ZERO, Color32::from_rgba_unmultiplied(0x2B, 0x7C, 0xD3, 110));
            glyph(p, ur(r, 0.0, 0.0, 0.35, 0.35), "❄", 0.9, BLUE);
        }
        Art::Split => {
            p.rect_stroke(ur(r, 0.1, 0.1, 0.9, 0.9), CornerRadius::same(1), s, StrokeKind::Inside);
            line(p, u(r, 0.5, 0.1), u(r, 0.5, 0.9), s);
            line(p, u(r, 0.1, 0.5), u(r, 0.9, 0.5), s);
        }
        Art::HideWindow => {
            p.rect_stroke(ur(r, 0.1, 0.1, 0.9, 0.9), CornerRadius::same(1), s, StrokeKind::Inside);
            line(p, u(r, 0.1, 0.9), u(r, 0.9, 0.1), s);
        }
        Art::UnhideWindow => {
            p.rect_stroke(ur(r, 0.1, 0.1, 0.9, 0.9), CornerRadius::same(1), s, StrokeKind::Inside);
            p.rect_filled(ur(r, 0.1, 0.1, 0.9, 0.25), CornerRadius::ZERO, ink);
        }
        Art::SideBySide | Art::SyncScroll | Art::ResetPosition => {
            p.rect_stroke(ur(r, 0.05, 0.15, 0.48, 0.85), CornerRadius::same(1), s, StrokeKind::Inside);
            p.rect_stroke(ur(r, 0.52, 0.15, 0.95, 0.85), CornerRadius::same(1), s, StrokeKind::Inside);
        }
        Art::SwitchWindows => {
            p.rect_stroke(ur(r, 0.25, 0.05, 0.95, 0.6), CornerRadius::same(1), s, StrokeKind::Inside);
            p.rect_filled(ur(r, 0.05, 0.35, 0.75, 0.95), CornerRadius::same(1), PAPER);
            p.rect_stroke(ur(r, 0.05, 0.35, 0.75, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            path(p, vec![u(r, 0.3, 0.75), u(r, 0.6, 0.75)], Stroke::new(w, BLUE));
        }
        Art::Macros => {
            sheet(p, ur(r, 0.05, 0.05, 0.85, 0.85), ink, Color32::from_gray(225), 3, 2);
            p.rect_filled(ur(r, 0.5, 0.55, 0.98, 0.98), CornerRadius::same(2), RED);
            lines(p, ur(r, 0.58, 0.62, 0.9, 0.92), 2, PAPER);
        }
        // ------------------------------------------------------------------ Automate / Developer
        Art::NewScript | Art::ViewScripts | Art::ScriptItem => {
            let body = ur(r, 0.15, 0.05, 0.8, 0.9);
            p.rect_filled(body, CornerRadius::same(2), if art == Art::ScriptItem { RED } else { PAPER });
            p.rect_stroke(body, CornerRadius::same(2), Stroke::new(w, if art == Art::ScriptItem { RED } else { ink }), StrokeKind::Inside);
            lines(p, ur(r, 0.25, 0.25, 0.7, 0.75), 3, if art == Art::ScriptItem { PAPER } else { GRAY });
            if art == Art::NewScript {
                line(p, u(r, 0.82, 0.55), u(r, 0.82, 0.98), Stroke::new(w * 1.3, GREEN));
                line(p, u(r, 0.6, 0.77), u(r, 1.0, 0.77), Stroke::new(w * 1.3, GREEN));
            }
            if art == Art::ViewScripts {
                lines(p, ur(r, 0.6, 0.6, 1.0, 0.98), 3, BLUE);
            }
        }
        Art::AutomationTemplates => {
            poly(p, vec![u(r, 0.1, 0.2), u(r, 0.45, 0.2), u(r, 0.75, 0.5), u(r, 0.45, 0.8), u(r, 0.1, 0.8), u(r, 0.4, 0.5)], GRAY, Stroke::NONE);
        }
        Art::VisualBasic => {
            for (x, y, c) in [(0.05, 0.4, BLUE), (0.4, 0.1, PURPLE), (0.4, 0.55, GREEN)] {
                p.rect_filled(ur(r, x, y, x + 0.4, y + 0.38), CornerRadius::same(2), c);
            }
        }
        Art::RecordMacro => {
            p.rect_stroke(ur(r, 0.05, 0.1, 0.95, 0.9), CornerRadius::same(1), s, StrokeKind::Inside);
            p.circle_filled(u(r, 0.5, 0.5), r.width() * 0.2, RED);
        }
        Art::RelativeRefs => sheet(p, ur(r, 0.05, 0.1, 0.95, 0.9), ink, BLUE, 2, 3),
        Art::MacroSecurity => {
            poly(p, vec![u(r, 0.5, 0.05), u(r, 0.95, 0.9), u(r, 0.05, 0.9)], YELLOW, Stroke::new(w, ORANGE));
            glyph(p, ur(r, 0.2, 0.3, 0.8, 0.95), "!", 0.8, ink);
        }
        Art::AddIns => {
            let c = u(r, 0.5, 0.5);
            let pts: Vec<Pos2> = (0..6)
                .map(|i| {
                    let a = std::f32::consts::FRAC_PI_3 * i as f32 + std::f32::consts::FRAC_PI_6;
                    pos2(c.x + r.width() * 0.42 * a.cos(), c.y + r.height() * 0.42 * a.sin())
                })
                .collect();
            let mut outline = pts.clone();
            outline.push(pts[0]);
            path(p, outline, Stroke::new(w * 1.2, BLUE));
        }
        Art::ExcelAddIns => glyph(p, r, "⚙", 0.9, ink),
        Art::ComAddIns => {
            page(p, ur(r, 0.1, 0.05, 0.75, 0.95), ink);
            glyph(p, ur(r, 0.45, 0.45, 1.0, 1.0), "⚙", 0.9, BLUE);
        }
        Art::InsertControl => {
            p.rect_filled(ur(r, 0.05, 0.35, 0.95, 0.9), CornerRadius::same(2), PAPER);
            p.rect_stroke(ur(r, 0.05, 0.35, 0.95, 0.9), CornerRadius::same(2), s, StrokeKind::Inside);
            path(p, vec![u(r, 0.35, 0.35), u(r, 0.35, 0.15), u(r, 0.65, 0.15), u(r, 0.65, 0.35)], s);
            line(p, u(r, 0.05, 0.55), u(r, 0.95, 0.55), s);
        }
        Art::DesignMode => {
            poly(p, vec![u(r, 0.05, 0.95), u(r, 0.05, 0.1), u(r, 0.9, 0.95)], PAPER, s);
        }
        Art::ControlProperties => lines(p, ur(r, 0.15, 0.15, 0.85, 0.85), 3, ink),
        Art::ViewCode => {
            page(p, ur(r, 0.1, 0.05, 0.75, 0.95), ink);
            magnifier(p, u(r, 0.6, 0.6), r.width() * 0.16, BLUE);
        }
        Art::RunDialog => {
            p.rect_stroke(ur(r, 0.05, 0.1, 0.95, 0.9), CornerRadius::same(1), s, StrokeKind::Inside);
            glyph(p, ur(r, 0.6, 0.4, 0.95, 0.9), "!", 0.9, RED);
            lines(p, ur(r, 0.15, 0.35, 0.55, 0.8), 3, GRAY);
        }
        Art::XmlSource => {
            p.rect_stroke(ur(r, 0.05, 0.1, 0.95, 0.9), CornerRadius::same(1), s, StrokeKind::Inside);
            p.rect_filled(ur(r, 0.25, 0.25, 0.55, 0.45), CornerRadius::ZERO, BLUE);
            p.rect_filled(ur(r, 0.4, 0.55, 0.75, 0.75), CornerRadius::ZERO, BLUE);
        }
        Art::MapProperties | Art::ExpansionPacks | Art::RefreshData => {
            sheet(p, ur(r, 0.05, 0.1, 0.95, 0.9), ink, Color32::from_gray(225), 2, 2);
            glyph(p, ur(r, 0.3, 0.35, 0.95, 0.95), "<>", 0.55, BLUE);
        }
        Art::Import | Art::Export => {
            sheet(p, ur(r, 0.3, 0.1, 0.95, 0.9), ink, Color32::from_gray(225), 2, 2);
            let (a, b) = if art == Art::Import { (u(r, 0.0, 0.5), u(r, 0.4, 0.5)) } else { (u(r, 0.4, 0.5), u(r, 0.0, 0.5)) };
            line(p, a, b, Stroke::new(w * 1.3, BLUE));
            poly(
                p,
                vec![pos2(b.x, b.y - 3.0), pos2(b.x, b.y + 3.0), pos2(b.x + if art == Art::Import { 3.0 } else { -3.0 }, b.y)],
                BLUE,
                Stroke::NONE,
            );
        }
        Art::Palette => {
            p.rect_stroke(ur(r, 0.05, 0.15, 0.95, 0.85), CornerRadius::same(3), s, StrokeKind::Inside);
            glyph(p, ur(r, 0.05, 0.15, 0.95, 0.85), ">_", 0.6, DARK_GREEN);
        }
        Art::Journal => {
            page(p, ur(r, 0.15, 0.05, 0.85, 0.95), ink);
            lines(p, ur(r, 0.25, 0.3, 0.75, 0.85), 4, BLUE);
        }
        Art::Agent => {
            p.rect_stroke(ur(r, 0.15, 0.25, 0.85, 0.85), CornerRadius::same(4), s, StrokeKind::Inside);
            p.circle_filled(u(r, 0.38, 0.52), r.width() * 0.06, BLUE);
            p.circle_filled(u(r, 0.62, 0.52), r.width() * 0.06, BLUE);
            line(p, u(r, 0.5, 0.05), u(r, 0.5, 0.25), s);
        }
        Art::About => {
            p.circle_stroke(u(r, 0.5, 0.5), r.width() * 0.42, Stroke::new(w, BLUE));
            glyph(p, r, "i", 0.6, BLUE);
        }
        // ------------------------------------------------------------------ Help
        Art::ContactSupport => {
            p.circle_stroke(u(r, 0.5, 0.35), r.width() * 0.2, s);
            path(p, vec![u(r, 0.15, 0.95), u(r, 0.25, 0.65), u(r, 0.75, 0.65), u(r, 0.85, 0.95)], s);
            path(p, vec![u(r, 0.25, 0.35), u(r, 0.25, 0.15), u(r, 0.75, 0.15), u(r, 0.75, 0.35)], Stroke::new(w, ink));
        }
        Art::Feedback => {
            p.circle_stroke(u(r, 0.35, 0.4), r.width() * 0.17, s);
            path(p, vec![u(r, 0.05, 0.95), u(r, 0.15, 0.68), u(r, 0.55, 0.68), u(r, 0.65, 0.95)], s);
            bubble(p, ur(r, 0.55, 0.05, 1.0, 0.45), ink, PAPER);
        }
        Art::ShowTraining => {
            poly(p, vec![u(r, 0.5, 0.15), u(r, 0.98, 0.38), u(r, 0.5, 0.6), u(r, 0.02, 0.38)], PAPER, s);
            path(p, vec![u(r, 0.25, 0.5), u(r, 0.25, 0.75), u(r, 0.5, 0.85), u(r, 0.75, 0.75), u(r, 0.75, 0.5)], s);
        }
        Art::WhatsNew => {
            poly(p, vec![u(r, 0.1, 0.45), u(r, 0.65, 0.15), u(r, 0.65, 0.85), u(r, 0.1, 0.65)], PAPER, s);
            for (a, b) in [((0.78, 0.25), (0.95, 0.1)), ((0.8, 0.5), (0.98, 0.5)), ((0.78, 0.75), (0.95, 0.9))] {
                line(p, u(r, a.0, a.1), u(r, b.0, b.1), Stroke::new(w, BLUE));
            }
        }
        Art::Community => {
            for x in [0.25, 0.5, 0.75] {
                p.circle_stroke(u(r, x, 0.35), r.width() * 0.11, s);
            }
            path(p, vec![u(r, 0.05, 0.9), u(r, 0.15, 0.6), u(r, 0.85, 0.6), u(r, 0.95, 0.9)], s);
        }
        Art::Blog => {
            p.rect_stroke(ur(r, 0.05, 0.05, 0.95, 0.95), CornerRadius::same(1), s, StrokeKind::Inside);
            p.rect_filled(ur(r, 0.15, 0.15, 0.85, 0.35), CornerRadius::ZERO, Color32::from_gray(200));
            lines(p, ur(r, 0.15, 0.45, 0.85, 0.85), 3, GRAY);
        }
        Art::MobileApp => {
            p.rect_stroke(ur(r, 0.28, 0.05, 0.72, 0.95), CornerRadius::same(3), s, StrokeKind::Inside);
            line(p, u(r, 0.44, 0.88), u(r, 0.56, 0.88), s);
        }
        Art::PdfXps => {
            p.rect_stroke(ur(r, 0.1, 0.35, 0.9, 0.75), CornerRadius::same(2), s, StrokeKind::Inside);
            p.rect_filled(ur(r, 0.25, 0.05, 0.75, 0.35), CornerRadius::ZERO, PAPER);
            p.rect_stroke(ur(r, 0.25, 0.05, 0.75, 0.35), CornerRadius::ZERO, s, StrokeKind::Inside);
            p.rect_filled(ur(r, 0.25, 0.65, 0.75, 0.95), CornerRadius::ZERO, PAPER);
            p.rect_stroke(ur(r, 0.25, 0.65, 0.75, 0.95), CornerRadius::ZERO, s, StrokeKind::Inside);
        }
    }
}
