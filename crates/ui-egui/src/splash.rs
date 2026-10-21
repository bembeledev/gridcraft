//! Application splash and loading screen.
//!
//! Renders an original branded splash screen on initial startup with an animated
//! indeterminate progress bar, status message, and smooth fade-out transition.
//! Both native desktop (`gridcraft`) and browser (`gridcraft-web`) use this to present
//! a polished loading experience while the workbook and fonts initialize.

use egui::epaint::{PathShape, PathStroke};
use egui::{Color32, CornerRadius, Pos2, Rect, Stroke, Ui, pos2, vec2};

use crate::SheetApp;
use crate::theme::{self, Tokens};

/// State of the initial application splash and loading screen.
#[derive(Clone, Debug)]
pub struct SplashState {
    /// Timestamp when the splash screen was initialized (milliseconds).
    pub start_ms: f64,
    /// Minimum time in milliseconds the splash screen stays fully visible before fading out.
    pub min_duration_ms: f64,
    /// Duration of the fade-out animation in milliseconds.
    pub fade_duration_ms: f64,
    /// Timestamp when dismissal began, if active.
    pub dismiss_ms: Option<f64>,
    /// Status message shown beneath the progress bar.
    pub status: String,
}

impl Default for SplashState {
    fn default() -> Self {
        Self::new()
    }
}

impl SplashState {
    /// Creates a new splash state initialized with default durations (start_ms begins on first paint).
    pub fn new() -> Self {
        Self { start_ms: 0.0, min_duration_ms: 1000.0, fade_duration_ms: 250.0, dismiss_ms: None, status: "Starting GridCraft…".to_string() }
    }

    /// Sets the status message displayed on the splash screen.
    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
    }

    /// Triggers dismissal of the splash screen at the specified timestamp.
    pub fn dismiss(&mut self, now: f64) {
        if self.dismiss_ms.is_none() {
            self.dismiss_ms = Some(now);
        }
    }

    /// Returns whether the splash screen is still active (visible or fading out).
    pub fn is_active(&self, now: f64) -> bool {
        self.alpha(now) > 0.0
    }

    /// Computes the current opacity factor (from 1.0 down to 0.0).
    pub fn alpha(&self, now: f64) -> f32 {
        if let Some(dismiss_start) = self.dismiss_ms {
            let elapsed = now - dismiss_start;
            if elapsed <= 0.0 {
                1.0
            } else if elapsed >= self.fade_duration_ms {
                0.0
            } else {
                (1.0 - (elapsed / self.fade_duration_ms) as f32).clamp(0.0, 1.0)
            }
        } else {
            1.0
        }
    }
}

/// Helper for drawing points rotated around a center point on a 512x512 design box.
struct LogoCanvas {
    origin: Pos2,
    scale: f32,
    cos_rad: f32,
    sin_rad: f32,
    center_x: f32,
    center_y: f32,
}

impl LogoCanvas {
    fn new(rect: Rect, angle_degrees: f32, center_x: f32, center_y: f32) -> Self {
        let rad = angle_degrees.to_radians();
        Self { origin: rect.min, scale: rect.width() / 512.0, cos_rad: rad.cos(), sin_rad: rad.sin(), center_x, center_y }
    }

    /// Rotates `(x, y)` around the center point and projects to screen coordinates.
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        let dx = x - self.center_x;
        let dy = y - self.center_y;
        let rx = self.center_x + dx * self.cos_rad - dy * self.sin_rad;
        let ry = self.center_y + dx * self.sin_rad + dy * self.cos_rad;
        pos2(self.origin.x + rx * self.scale, self.origin.y + ry * self.scale)
    }
}

/// Multiplies a color's alpha channel by an opacity multiplier in `[0.0, 1.0]`.
fn apply_alpha(color: Color32, alpha: f32) -> Color32 {
    let a = (color.a() as f32 * alpha.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgba_premultiplied(
        (color.r() as f32 * (a as f32 / 255.0)).round() as u8,
        (color.g() as f32 * (a as f32 / 255.0)).round() as u8,
        (color.b() as f32 * (a as f32 / 255.0)).round() as u8,
        a,
    )
}

/// Paints the official GridCraft vector logo within `rect`.
///
/// Draws original vector artwork matching `assets/app-icon/hicolor/scalable/apps/ai.storyteller.gridcraft.svg`:
/// a rounded emerald tile with a tilted ledger page, green header row, grid matrix,
/// and selected cell indicator.
pub fn paint_app_logo(p: &egui::Painter, rect: Rect, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let side = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center(), vec2(side, side));
    let s = side / 512.0;

    // Palette
    let green_tile = Color32::from_rgb(0x1F, 0x9D, 0x55);
    let paper = Color32::from_rgb(0xEF, 0xE9, 0xDC);
    let ink = Color32::from_rgb(0x0B, 0x0B, 0x0C);

    // 1. Tile background with rounded corners (rx=112 on 512 grid -> 112/512 * side)
    let corner_radius = CornerRadius::same((112.0 * s).round() as u8);
    p.rect_filled(r, corner_radius, apply_alpha(green_tile, alpha));

    // Logo canvas with -6 deg rotation around (256, 268)
    let lc = LogoCanvas::new(r, -6.0, 256.0, 268.0);

    // 2. Paper page with folded top-right corner: M112 76 H336 L404 144 V440 H112 Z
    let page_pts = vec![lc.pt(112.0, 76.0), lc.pt(336.0, 76.0), lc.pt(404.0, 144.0), lc.pt(404.0, 440.0), lc.pt(112.0, 440.0)];
    p.add(PathShape::convex_polygon(page_pts.clone(), apply_alpha(paper, alpha), Stroke::NONE));
    p.add(PathShape::line(
        vec![lc.pt(112.0, 76.0), lc.pt(336.0, 76.0), lc.pt(404.0, 144.0), lc.pt(404.0, 440.0), lc.pt(112.0, 440.0), lc.pt(112.0, 76.0)],
        PathStroke::new((14.0 * s).max(1.0), apply_alpha(ink, alpha)),
    ));

    // 3. Corner fold: M336 76 V144 H404
    p.add(PathShape::line(
        vec![lc.pt(336.0, 76.0), lc.pt(336.0, 144.0), lc.pt(404.0, 144.0)],
        PathStroke::new((12.0 * s).max(1.0), apply_alpha(ink, alpha)),
    ));

    // 4. Title lines: M146 106 H272, M146 130 H232
    p.add(PathShape::line(vec![lc.pt(146.0, 106.0), lc.pt(272.0, 106.0)], PathStroke::new((12.0 * s).max(1.0), apply_alpha(ink, alpha))));
    p.add(PathShape::line(vec![lc.pt(146.0, 130.0), lc.pt(232.0, 130.0)], PathStroke::new((12.0 * s).max(1.0), apply_alpha(ink, alpha))));

    // 5. Header row and row-number column in green:
    // M112 164 H404 V204 H164 V440 H112 Z
    // Split into two convex rects for safe rendering with convex_polygon:
    // Top banner: (112, 164) to (404, 204)
    let banner_pts = vec![lc.pt(112.0, 164.0), lc.pt(404.0, 164.0), lc.pt(404.0, 204.0), lc.pt(112.0, 204.0)];
    p.add(PathShape::convex_polygon(banner_pts, apply_alpha(green_tile, alpha), Stroke::NONE));
    // Side column: (112, 204) to (164, 440)
    let col_pts = vec![lc.pt(112.0, 204.0), lc.pt(164.0, 204.0), lc.pt(164.0, 440.0), lc.pt(112.0, 440.0)];
    p.add(PathShape::convex_polygon(col_pts, apply_alpha(green_tile, alpha), Stroke::NONE));

    // 6. Grid lines (stroke width 8)
    let grid_stroke = PathStroke::new((8.0 * s).max(0.75), apply_alpha(ink, alpha));
    for y in [164.0, 204.0, 250.0, 296.0, 342.0, 388.0] {
        p.add(PathShape::line(vec![lc.pt(112.0, y), lc.pt(404.0, y)], grid_stroke.clone()));
    }
    for x in [164.0, 244.0, 324.0] {
        p.add(PathShape::line(vec![lc.pt(x, 164.0), lc.pt(x, 440.0)], grid_stroke.clone()));
    }

    // 7. Selected cell at (244, 250) size 80x46 (stroke width 16)
    let cell_stroke = PathStroke::new((16.0 * s).max(1.5), apply_alpha(ink, alpha));
    p.add(PathShape::line(
        vec![lc.pt(244.0, 250.0), lc.pt(324.0, 250.0), lc.pt(324.0, 296.0), lc.pt(244.0, 296.0), lc.pt(244.0, 250.0)],
        cell_stroke,
    ));

    // 8. Fill handle at (312, 284) size 24x24
    let handle_pts = vec![lc.pt(312.0, 284.0), lc.pt(336.0, 284.0), lc.pt(336.0, 308.0), lc.pt(312.0, 308.0)];
    p.add(PathShape::convex_polygon(handle_pts, apply_alpha(ink, alpha), Stroke::NONE));
    p.add(PathShape::line(
        vec![lc.pt(312.0, 284.0), lc.pt(336.0, 284.0), lc.pt(336.0, 308.0), lc.pt(312.0, 308.0), lc.pt(312.0, 284.0)],
        PathStroke::new((5.0 * s).max(0.75), apply_alpha(paper, alpha)),
    ));

    // 9. Double rule under total: (252, 410) H316 and (252, 424) H316
    let rule_stroke = PathStroke::new((7.0 * s).max(0.75), apply_alpha(ink, alpha));
    p.add(PathShape::line(vec![lc.pt(252.0, 410.0), lc.pt(316.0, 410.0)], rule_stroke.clone()));
    p.add(PathShape::line(vec![lc.pt(252.0, 424.0), lc.pt(316.0, 424.0)], rule_stroke));
}

/// Displays the splash loading screen overlay if active on `app`.
pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    let now = crate::now_ms();

    // Check if splash state is set
    let Some(splash) = &mut app.splash else { return };

    // Record timestamp when the splash is first painted
    if splash.start_ms <= 0.0 {
        splash.start_ms = now;
    }

    let alpha = splash.alpha(now);
    if alpha <= 0.0 {
        app.splash = None;
        return;
    }

    // Request continuous repaint while splash is active or fading
    ui.ctx().request_repaint();

    let screen = ui.max_rect();
    let t = Tokens::get(ui.ctx());
    let mut clicked = false;

    egui::Area::new(egui::Id::new("splash_overlay")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ui.ctx(), |ui| {
        let resp = ui.allocate_response(screen.size(), egui::Sense::click());
        if resp.clicked() {
            clicked = true;
        }
        let p = ui.painter();

        // 1. Semi-transparent backdrop overlay over entire window
        let backdrop = if t.dark { Color32::from_black_alpha((160.0 * alpha) as u8) } else { Color32::from_black_alpha((70.0 * alpha) as u8) };
        p.rect_filled(screen, CornerRadius::ZERO, backdrop);

        // 2. Centered splash card
        let card_w = (screen.width() - 32.0).clamp(280.0, 480.0);
        let card_h = 240.0;
        let card_rect = Rect::from_center_size(screen.center(), vec2(card_w, card_h));

        // Card drop shadow
        let shadow_color = Color32::from_black_alpha((40.0 * alpha) as u8);
        p.rect_filled(card_rect.translate(vec2(0.0, 6.0)), CornerRadius::same(12), shadow_color);

        // Card background and border
        let card_bg = if t.dark { Color32::from_rgb(0x2B, 0x2B, 0x2B) } else { Color32::WHITE };
        p.rect_filled(card_rect, CornerRadius::same(12), apply_alpha(card_bg, alpha));

        let border_color = if t.dark { Color32::from_white_alpha((28.0 * alpha) as u8) } else { Color32::from_black_alpha((20.0 * alpha) as u8) };
        p.rect_stroke(card_rect, CornerRadius::same(12), Stroke::new(1.0, border_color), egui::StrokeKind::Inside);

        // 3. GridCraft App Logo (left side of card)
        let logo_size = 76.0;
        let logo_rect = Rect::from_min_size(pos2(card_rect.left() + 28.0, card_rect.top() + 34.0), vec2(logo_size, logo_size));
        paint_app_logo(p, logo_rect, alpha);

        // 4. Content (right side of card)
        let content_left = card_rect.left() + 124.0;
        let content_w = (card_rect.right() - content_left - 28.0).max(100.0);

        // Title: "GridCraft"
        let title_color = if t.dark { Color32::from_rgb(0x5C, 0xC5, 0x87) } else { Color32::from_rgb(0x10, 0x7C, 0x41) };
        p.text(
            pos2(content_left, card_rect.top() + 38.0),
            egui::Align2::LEFT_TOP,
            "GridCraft",
            theme::ui_bold(26.0),
            apply_alpha(title_color, alpha),
        );

        // Subtitle
        p.text(
            pos2(content_left, card_rect.top() + 72.0),
            egui::Align2::LEFT_TOP,
            "Clean-room, native spreadsheet",
            theme::ui_font(12.5),
            apply_alpha(t.text_dim, alpha),
        );

        // Indeterminate Progress Bar
        let track_rect = Rect::from_min_size(pos2(content_left, card_rect.top() + 112.0), vec2(content_w, 4.0));
        let track_bg = if t.dark { Color32::from_rgb(0x40, 0x40, 0x40) } else { Color32::from_rgb(0xE0, 0xE0, 0xE0) };
        p.rect_filled(track_rect, CornerRadius::same(2), apply_alpha(track_bg, alpha));

        // Smooth indeterminate animation
        let elapsed = ((now - splash.start_ms).max(0.0) / 1000.0) as f32;
        let bar_w = (content_w * 0.35).clamp(40.0, 90.0);
        // Smooth oscillating progress between 0.0 and 1.0
        let phase = (0.5 - 0.5 * (elapsed * std::f32::consts::PI * 1.1).cos()).clamp(0.0, 1.0);
        let bar_left = track_rect.left() + phase * (content_w - bar_w);
        let bar_rect = Rect::from_min_size(pos2(bar_left, track_rect.top()), vec2(bar_w, 4.0));
        let accent_bar = if t.dark { Color32::from_rgb(0x5C, 0xC5, 0x87) } else { Color32::from_rgb(0x10, 0x7C, 0x41) };
        p.rect_filled(bar_rect, CornerRadius::same(2), apply_alpha(accent_bar, alpha));

        // Status text
        p.text(
            pos2(content_left, card_rect.top() + 128.0),
            egui::Align2::LEFT_TOP,
            &splash.status,
            theme::ui_font(12.0),
            apply_alpha(t.text_dim, alpha),
        );

        // Hint at bottom center of card
        p.text(
            pos2(card_rect.center().x, card_rect.bottom() - 18.0),
            egui::Align2::CENTER_CENTER,
            "Click or press any key to continue",
            theme::ui_font(10.5),
            apply_alpha(t.text_disabled, alpha * 0.75),
        );
    });

    // Check for user dismiss interaction: pointer click on overlay or a pressed-key event
    let key_pressed = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
    if clicked || key_pressed {
        splash.dismiss(now);
    }

    // Auto-dismiss after min_duration_ms has elapsed
    if now - splash.start_ms >= splash.min_duration_ms && splash.dismiss_ms.is_none() {
        splash.dismiss(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splash_state_lifecycle() {
        let mut splash = SplashState::new();
        splash.start_ms = 1000.0;
        splash.min_duration_ms = 1000.0;
        splash.fade_duration_ms = 250.0;

        // Fully opaque during min duration
        assert_eq!(splash.alpha(1500.0), 1.0);
        assert!(splash.is_active(1500.0));

        // Start dismissal
        splash.dismiss(2000.0);
        assert_eq!(splash.alpha(2000.0), 1.0);

        // Halfway through fade
        let mid_alpha = splash.alpha(2125.0);
        assert!((mid_alpha - 0.5).abs() < 0.05);
        assert!(splash.is_active(2125.0));

        // After fade duration
        assert_eq!(splash.alpha(2250.0), 0.0);
        assert!(!splash.is_active(2250.0));
    }

    #[test]
    fn splash_status_update() {
        let mut splash = SplashState::new();
        assert_eq!(splash.status, "Starting GridCraft…");
        splash.set_status("Opening Sales.xlsx…");
        assert_eq!(splash.status, "Opening Sales.xlsx…");
    }

    #[test]
    fn splash_active_returns_early_in_ui() {
        let mut session = gridcraft_engine::Session::default();
        session.new_workbook();
        let mut app = SheetApp::new(session, Default::default());
        app.enable_splash();
        let ctx = egui::Context::default();
        app.logic(&ctx);
        app.logic(&ctx);
        assert!(app.splash.is_some());
        let mut output = ctx.run_ui(Default::default(), |ui| {
            app.ui(ui);
        });
        output.textures_delta.clear();

        assert!(app.splash.is_some());
        assert_eq!(app.perf.grid_ms, 0.0);
    }

    #[test]
    fn splash_dismissal_on_keypress() {
        let mut session = gridcraft_engine::Session::default();
        session.new_workbook();
        let mut app = SheetApp::new(session, Default::default());
        app.enable_splash();
        let ctx = egui::Context::default();
        app.logic(&ctx);
        app.logic(&ctx);
        let mut raw_input = egui::RawInput::default();
        raw_input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        });

        let mut output = ctx.run_ui(raw_input, |ui| {
            app.ui(ui);
        });
        output.textures_delta.clear();

        let splash = app.splash.as_ref().expect("splash still present during fade");
        assert!(splash.dismiss_ms.is_some());
        assert_eq!(app.perf.grid_ms, 0.0);
    }
}
