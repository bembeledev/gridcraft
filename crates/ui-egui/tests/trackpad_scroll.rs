//! Trackpad swipes are held to the axis they start along: scrolling down a column doesn't drift
//! sideways, scrolling along a row doesn't drift up or down, and a diagonal swipe moves freely.

use egui::{Event, Modifiers, MouseWheelUnit, TouchPhase, Vec2, vec2};
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(4);
    let over_grid = h.state().grid.rect.unwrap().center();
    h.input_mut().events.push(Event::PointerMoved(over_grid));
    h.run_steps(2);
    h
}

fn scroll(h: &Harness) -> Vec2 {
    h.state().view().scroll
}

fn wheel(h: &mut Harness, phase: TouchPhase, delta: Vec2) {
    h.input_mut().events.push(Event::MouseWheel { unit: MouseWheelUnit::Point, delta, phase, modifiers: Modifiers::NONE });
    h.step();
}

/// A trackpad gesture: fingers down, `frames` frames moving `delta` each, fingers up. Negative
/// deltas scroll down and right.
fn swipe(h: &mut Harness, delta: Vec2, frames: usize) {
    wheel(h, TouchPhase::Start, Vec2::ZERO);
    for _ in 0..frames {
        wheel(h, TouchPhase::Move, delta);
    }
    wheel(h, TouchPhase::End, Vec2::ZERO);
}

#[test]
fn a_mostly_vertical_swipe_does_not_drift_sideways() {
    let mut h = harness();
    let before = scroll(&h);
    // About 7° off vertical, as fingers drift.
    swipe(&mut h, vec2(-1.5, -12.0), 12);
    // Momentum after the fingers lift (no new gesture) keeps to the same axis.
    for _ in 0..6 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.0, -6.0));
    }
    let after = scroll(&h);
    assert!(after.y > before.y + 100.0, "scrolled down: {before:?} -> {after:?}");
    assert_eq!(after.x, before.x, "no sideways drift: {before:?} -> {after:?}");
}

#[test]
fn a_mostly_horizontal_swipe_does_not_drift_up_or_down() {
    let mut h = harness();
    let before = scroll(&h);
    swipe(&mut h, vec2(-12.0, -1.5), 12);
    let after = scroll(&h);
    assert!(after.x > before.x + 100.0, "scrolled right: {before:?} -> {after:?}");
    assert_eq!(after.y, before.y, "no vertical drift: {before:?} -> {after:?}");
}

#[test]
fn a_diagonal_swipe_moves_both_ways() {
    let mut h = harness();
    let before = scroll(&h);
    swipe(&mut h, vec2(-8.0, -8.0), 12);
    let after = scroll(&h);
    assert!(after.x > before.x + 50.0 && after.y > before.y + 50.0, "{before:?} -> {after:?}");
}

#[test]
fn each_swipe_picks_its_own_axis() {
    let mut h = harness();
    swipe(&mut h, vec2(-1.0, -12.0), 10);
    // Without gesture phases (as on the web), a pause ends the swipe.
    h.run_steps(20);
    let before = scroll(&h);
    for _ in 0..10 {
        wheel(&mut h, TouchPhase::Move, vec2(-12.0, -1.0));
    }
    let after = scroll(&h);
    assert!(after.x > before.x + 100.0, "the second swipe goes right: {before:?} -> {after:?}");
    assert_eq!(after.y, before.y, "and only right: {before:?} -> {after:?}");
    // A new gesture starts a new swipe at once, even straight after the last one.
    let before = scroll(&h);
    swipe(&mut h, vec2(-1.0, -12.0), 10);
    let after = scroll(&h);
    assert!(after.y > before.y + 100.0 && after.x == before.x, "{before:?} -> {after:?}");
}

#[test]
fn shift_turns_a_vertical_wheel_sideways() {
    // Off the Mac, Shift+wheel arrives as vertical movement and egui turns it horizontal; the
    // axis rail must not hold it to the vertical axis it was reported on.
    let mut h = harness();
    let before = scroll(&h);
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::SHIFT));
    for _ in 0..3 {
        h.input_mut().events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, -2.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::SHIFT,
        });
        h.run_steps(4);
    }
    h.run_steps(30);
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
    let after = scroll(&h);
    assert!(after.x > before.x + 50.0, "Shift+wheel scrolls right: {before:?} -> {after:?}");
    assert_eq!(after.y, before.y, "and not down: {before:?} -> {after:?}");
}

#[test]
fn a_short_diagonal_swipe_keeps_both_directions() {
    // Before a swipe has gone far enough to pick an axis, a diagonal movement moves both ways.
    let mut h = harness();
    let before = scroll(&h);
    swipe(&mut h, vec2(-3.0, -3.0), 1);
    let after = scroll(&h);
    assert!(after.x > before.x && after.y > before.y, "{before:?} -> {after:?}");
}
