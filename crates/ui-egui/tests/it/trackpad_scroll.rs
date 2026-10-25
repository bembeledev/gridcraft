//! Trackpad gestures are held to the axis they start along: scrolling down a column doesn't drift
//! sideways, scrolling along a row doesn't drift up or down, and a diagonal swipe moves freely.
//! Only gestures the trackpad reports with a start and an end are steered; scrolling without
//! gesture phases (the web, mouse wheels) passes untouched.

use egui::{Event, Modifiers, MouseWheelUnit, TouchPhase, Vec2, vec2};
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    harness_at(1.0 / 60.0)
}

/// A harness whose frames are `dt` seconds apart (a slow one stands in for a heavy workbook).
fn harness_at(dt: f32) -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(dt).build_ui_state(
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
    // The glide after the fingers lift arrives as a gesture of its own (macOS reports it so) and
    // keeps to the same axis.
    swipe(&mut h, vec2(-1.0, -6.0), 6);
    let after = scroll(&h);
    assert!(after.y > before.y + 100.0, "scrolled down: {before:?} -> {after:?}");
    assert_eq!(after.x, before.x, "no sideways drift: {before:?} -> {after:?}");
}

#[test]
fn a_mostly_horizontal_swipe_does_not_drift_up_or_down() {
    let mut h = harness();
    let before = scroll(&h);
    swipe(&mut h, vec2(-12.0, -1.5), 12);
    swipe(&mut h, vec2(-6.0, -1.0), 6);
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
fn a_short_diagonal_swipe_keeps_both_directions() {
    // Before a swipe has gone far enough to pick an axis, a diagonal movement moves both ways.
    let mut h = harness();
    let before = scroll(&h);
    swipe(&mut h, vec2(-3.0, -3.0), 1);
    let after = scroll(&h);
    assert!(after.x > before.x && after.y > before.y, "{before:?} -> {after:?}");
}

#[test]
fn each_gesture_picks_its_own_axis() {
    // Gestures in a row, each with its start and end: a horizontal one after a vertical one is
    // held to its own axis, and a diagonal one after that moves both ways.
    let mut h = harness();
    swipe(&mut h, vec2(-1.0, -12.0), 10);
    let before = scroll(&h);
    swipe(&mut h, vec2(-12.0, -1.0), 10);
    let after = scroll(&h);
    assert!(after.x > before.x + 100.0, "the second gesture goes right: {before:?} -> {after:?}");
    assert_eq!(after.y, before.y, "and only right: {before:?} -> {after:?}");
    let before = scroll(&h);
    swipe(&mut h, vec2(-8.0, -8.0), 10);
    let after = scroll(&h);
    assert!(after.y > before.y + 50.0 && after.x > before.x + 50.0, "a diagonal gesture moves both ways: {before:?} -> {after:?}");
}

#[test]
fn a_glide_reported_as_its_own_gesture_picks_its_own_axis() {
    // macOS reports the momentum after the fingers lift as a gesture of its own (start, moves,
    // end), which the rail can't tell from fingers touching down again. So a glide is a swipe of
    // its own: one that keeps the swipe's direction stays on its axis, a diagonal one moves freely.
    let mut h = harness();
    swipe(&mut h, vec2(-1.5, -12.0), 12);
    let before = scroll(&h);
    swipe(&mut h, vec2(-1.0, -6.0), 6);
    let after = scroll(&h);
    assert!(after.y > before.y + 30.0, "the glide scrolls on: {before:?} -> {after:?}");
    assert_eq!(after.x, before.x, "a glide along the swipe stays on its axis: {before:?} -> {after:?}");
    h.run_steps(20);
    swipe(&mut h, vec2(-1.5, -12.0), 12);
    let before = scroll(&h);
    swipe(&mut h, vec2(-5.0, -8.0), 6);
    let after = scroll(&h);
    assert!(after.x > before.x && after.y > before.y, "a diagonal glide moves both ways: {before:?} -> {after:?}");
}

#[test]
fn a_swipe_starting_mid_frame_decides_its_axis_from_its_own_movement() {
    // One frame can hold the end of one gesture and the start of the next (a slow frame on a
    // heavy workbook, or a quick lift and touch): the earlier gesture's last movement must not
    // decide the new gesture's axis. Here the new gesture is horizontal with a little drift; counted
    // with the vertical movement before its start it would come out diagonal and the drift would pass.
    let mut h = harness();
    wheel(&mut h, TouchPhase::Start, Vec2::ZERO);
    for _ in 0..6 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
    }
    let before = scroll(&h);
    for (phase, delta) in
        [(TouchPhase::Move, vec2(-1.5, -12.0)), (TouchPhase::End, Vec2::ZERO), (TouchPhase::Start, Vec2::ZERO), (TouchPhase::Move, vec2(-12.0, -1.5))]
    {
        h.input_mut().events.push(Event::MouseWheel { unit: MouseWheelUnit::Point, delta, phase, modifiers: Modifiers::NONE });
    }
    h.step();
    for _ in 0..8 {
        wheel(&mut h, TouchPhase::Move, vec2(-12.0, -1.5));
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(after.x > before.x + 50.0, "scrolled right: {before:?} -> {after:?}");
    assert_eq!(after.y, before.y, "the earlier gesture's movement does not steer the new one: {before:?} -> {after:?}");
}

#[test]
fn a_gesture_keeps_its_axis_through_a_cross_frame_and_a_rest() {
    // With gesture phases the trackpad says when a swipe ends: a frame across the axis inside
    // the gesture is drift, not a new swipe, and so is one after the fingers rested a while.
    let mut h = harness();
    let before = scroll(&h);
    wheel(&mut h, TouchPhase::Start, Vec2::ZERO);
    for _ in 0..6 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
    }
    wheel(&mut h, TouchPhase::Move, vec2(-12.0, -2.0));
    for _ in 0..3 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
    }
    // Fingers resting on the trackpad for longer than the pause that ends a phase-less swipe,
    // then a drift frame first thing.
    h.run_steps(20);
    wheel(&mut h, TouchPhase::Move, vec2(-12.0, -2.0));
    for _ in 0..3 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(after.y > before.y + 100.0, "scrolled down: {before:?} -> {after:?}");
    assert_eq!(after.x, before.x, "no sideways jump inside the gesture: {before:?} -> {after:?}");
}

#[test]
fn a_gesture_keeps_its_axis_when_it_wanders_later() {
    // The axis is decided from the gesture's first points and kept: fingers that start down a
    // column and wander diagonally later are still reading down the column.
    let mut h = harness();
    let before = scroll(&h);
    wheel(&mut h, TouchPhase::Start, Vec2::ZERO);
    for _ in 0..6 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
    }
    for _ in 0..10 {
        wheel(&mut h, TouchPhase::Move, vec2(-8.0, -8.0));
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(after.y > before.y + 100.0, "scrolled down: {before:?} -> {after:?}");
    assert_eq!(after.x, before.x, "held to the axis it started along: {before:?} -> {after:?}");
}

#[test]
fn a_gestures_end_and_the_next_start_in_one_frame_leave_a_gesture_in_progress() {
    // Fingers lifting and the glide starting can share a frame (macOS reports the glide as a
    // gesture of its own); applied in order, the start wins, so the glide is a gesture: a rest
    // and a drift frame inside it don't start a new swipe.
    let mut h = harness();
    swipe(&mut h, vec2(-1.5, -12.0), 6);
    wheel(&mut h, TouchPhase::Start, Vec2::ZERO);
    for _ in 0..6 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
    }
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: Vec2::ZERO,
        phase: TouchPhase::End,
        modifiers: Modifiers::NONE,
    });
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: Vec2::ZERO,
        phase: TouchPhase::Start,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for _ in 0..6 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.0, -6.0));
    }
    let before = scroll(&h);
    h.run_steps(20);
    wheel(&mut h, TouchPhase::Move, vec2(-12.0, -2.0));
    for _ in 0..3 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.0, -6.0));
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(after.y > before.y + 20.0, "the glide scrolls on: {before:?} -> {after:?}");
    assert_eq!(after.x, before.x, "held to its axis through the rest: {before:?} -> {after:?}");
}

#[test]
fn a_gesture_held_in_one_frame_still_ends() {
    // A slow frame can hold a whole gesture, start to end (a brief two-finger touch): the phases
    // apply in order, so the gesture is over and what follows without phases passes untouched.
    let mut h = harness();
    for (phase, delta) in [(TouchPhase::Start, Vec2::ZERO), (TouchPhase::Move, vec2(-1.5, -12.0)), (TouchPhase::End, Vec2::ZERO)] {
        h.input_mut().events.push(Event::MouseWheel { unit: MouseWheelUnit::Point, delta, phase, modifiers: Modifiers::NONE });
    }
    h.step();
    for _ in 0..6 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.0, -12.0));
    }
    let before = scroll(&h);
    for _ in 0..6 {
        wheel(&mut h, TouchPhase::Move, vec2(-12.0, -1.0));
    }
    let after = scroll(&h);
    assert!(after.x > before.x + 50.0, "the turn goes right: {before:?} -> {after:?}");
}

#[test]
fn a_large_step_inside_a_gesture_decides_its_axis_like_any_movement() {
    // A step lying on an axis, however large (a fast flick), is the gesture's own: it decides
    // the axis like any movement.
    let mut h = harness();
    let before = scroll(&h);
    wheel(&mut h, TouchPhase::Start, Vec2::ZERO);
    wheel(&mut h, TouchPhase::Move, vec2(0.0, -60.0));
    for _ in 0..6 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(after.y > before.y + 100.0, "scrolls down: {before:?} -> {after:?}");
    assert_eq!(after.x, before.x, "held to its axis: {before:?} -> {after:?}");
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
fn releasing_shift_mid_swipe_keeps_scrolling() {
    // egui keeps a gesture's modifiers after they're released (more of the gesture may arrive) and
    // turns its whole movement, drift included, onto one axis. The rail must pass that as it is,
    // not decide an axis from the raw movement and steer egui's turned scroll by it. The gesture
    // here goes sideways with a little vertical drift, which egui folds into the horizontal
    // scroll; a rail deciding Horizontal on its own would leave the drift's share scrolling up.
    let mut h = harness();
    // Away from the top, so an upward leak would show.
    swipe(&mut h, vec2(0.0, -12.0), 10);
    let before = scroll(&h);
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::SHIFT));
    wheel(&mut h, TouchPhase::Start, Vec2::ZERO);
    for _ in 0..5 {
        h.input_mut().events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(-12.0, -1.5),
            phase: TouchPhase::Move,
            modifiers: Modifiers::SHIFT,
        });
        h.step();
    }
    let midway = scroll(&h);
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
    for _ in 0..5 {
        wheel(&mut h, TouchPhase::Move, vec2(-12.0, -1.5));
    }
    let rested = scroll(&h);
    // Fingers resting mid-gesture, longer than egui's own end-of-scroll timer: the gesture keeps
    // its modifiers (egui does), so it keeps scrolling right afterwards too.
    h.run_steps(20);
    for _ in 0..5 {
        wheel(&mut h, TouchPhase::Move, vec2(-12.0, -1.5));
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(midway.x > before.x + 50.0, "Shift+gesture scrolls right: {before:?} -> {midway:?}");
    assert!(rested.x > midway.x + 50.0, "and keeps scrolling right after Shift is released: {midway:?} -> {rested:?}");
    assert!(after.x > rested.x + 50.0, "and after a rest: {rested:?} -> {after:?}");
    assert_eq!(after.y, before.y, "never up or down: {before:?} -> {after:?}");
}

#[test]
fn shift_held_at_a_gestures_start_and_released_before_it_moves_holds_the_gesture() {
    // egui keeps the modifiers a gesture starts with, so the whole gesture scrolls horizontally
    // even when Shift is released before the first move; the rail takes them from the start too.
    let mut h = harness();
    swipe(&mut h, vec2(0.0, -12.0), 10);
    let before = scroll(&h);
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::SHIFT));
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: Vec2::ZERO,
        phase: TouchPhase::Start,
        modifiers: Modifiers::SHIFT,
    });
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
    for _ in 0..10 {
        wheel(&mut h, TouchPhase::Move, vec2(-12.0, -1.5));
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(after.x > before.x + 50.0, "scrolls right: {before:?} -> {after:?}");
    assert_eq!(after.y, before.y, "and not up or down: {before:?} -> {after:?}");
}

#[test]
fn touching_the_trackpad_does_not_swallow_a_wheel_notch() {
    // A mouse-wheel notch is smoothed out over a few frames; fingers touching the trackpad right
    // after it start a swipe, but the notch is not that swipe's to cut.
    let notch_then = |touch: bool| {
        let mut h = harness();
        let before = scroll(&h);
        h.input_mut().events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, -3.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.step();
        if touch {
            wheel(&mut h, TouchPhase::Start, Vec2::ZERO);
        }
        h.run_steps(60);
        scroll(&h).y - before.y
    };
    let (alone, touched) = (notch_then(false), notch_then(true));
    assert!(alone > 100.0, "the notch scrolls: {alone}");
    assert_eq!(touched, alone, "the whole notch scrolls with fingers touching down after it");
}

#[test]
fn a_wheel_notch_in_the_frame_a_gesture_starts_does_not_free_the_gesture() {
    // A mouse-wheel notch in lines and a trackpad touch in one frame: the notch passes whole, which
    // takes a moment of smoothing, and the gesture that starts is held to its axis from then on.
    let mut h = harness();
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: vec2(0.0, -3.0),
        phase: TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: Vec2::ZERO,
        phase: TouchPhase::Start,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for _ in 0..12 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
    }
    let settled = scroll(&h);
    for _ in 0..8 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(after.y > settled.y + 50.0, "the gesture scrolls down: {settled:?} -> {after:?}");
    assert_eq!(after.x, settled.x, "and is held to its axis once the notch is through: {settled:?} -> {after:?}");
}

#[test]
fn a_wheel_notch_during_a_gesture_scrolls_its_own_way() {
    // A mouse-wheel tilt in lines while a vertical trackpad gesture is going (a mouse and a trackpad
    // side by side): the notch isn't the gesture's and scrolls sideways, as far as the same notch
    // alone, while the gesture stays held to its axis.
    let notch = |during_gesture: bool| {
        let mut h = harness();
        if during_gesture {
            wheel(&mut h, TouchPhase::Start, Vec2::ZERO);
            for _ in 0..6 {
                wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
            }
        }
        let before = scroll(&h);
        h.input_mut().events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(-3.0, 0.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.step();
        if during_gesture {
            for _ in 0..6 {
                wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
            }
            wheel(&mut h, TouchPhase::End, Vec2::ZERO);
        } else {
            h.run_steps(7);
        }
        h.run_steps(60);
        (scroll(&h) - before).x
    };
    let (alone, during) = (notch(false), notch(true));
    assert!(alone > 100.0, "the notch alone: {alone}");
    assert!((during - alone).abs() <= alone * 0.02, "the tilt notch scrolls sideways as far during a gesture: {during} vs {alone}");
}

#[test]
fn a_slow_frame_does_not_swallow_a_wheel_notch() {
    // A mouse-wheel notch in lines, smoothed by egui over frames longer than usual (a heavy
    // workbook): without gesture phases the rail leaves it alone, so it scrolls the same distance
    // as at 60 fps.
    let notch = |h: &mut Harness, frames: usize, delta: Vec2| {
        let before = scroll(h);
        h.input_mut().events.push(Event::MouseWheel { unit: MouseWheelUnit::Line, delta, phase: TouchPhase::Move, modifiers: Modifiers::NONE });
        h.run_steps(frames);
        scroll(h) - before
    };
    // A vertical notch and a horizontal one (a tilt wheel, or a wheel with Shift held off the
    // Mac, which egui has already turned sideways by the time the rail sees it).
    for delta in [vec2(0.0, -3.0), vec2(-3.0, 0.0)] {
        let fast = notch(&mut harness(), 60, delta);
        let slow = notch(&mut harness_at(0.3), 4, delta);
        assert!(fast.length() > 50.0, "a notch scrolls: {fast:?}");
        assert!((slow - fast).length() < 1.0, "the same notch {delta:?} over slow frames: {slow:?} vs {fast:?}");
    }
}

#[test]
fn without_gesture_phases_scrolling_is_untouched() {
    // Where no gesture phases arrive (the web, mouse wheels) nothing says where a swipe begins or
    // ends, so the rail leaves the scroll as it is: the small off-axis part of each movement
    // passes, as it does on main.
    let mut h = harness();
    let before = scroll(&h);
    for _ in 0..6 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
    }
    let after = scroll(&h);
    assert!(after.y > before.y + 50.0 && after.x > before.x, "both components pass: {before:?} -> {after:?}");
    let before = scroll(&h);
    for _ in 0..6 {
        wheel(&mut h, TouchPhase::Move, vec2(-12.0, -1.5));
    }
    let after = scroll(&h);
    assert!(after.x > before.x + 50.0 && after.y > before.y, "both components pass: {before:?} -> {after:?}");
}

#[test]
fn a_mouse_wheel_in_lines_is_not_held_to_an_axis() {
    // A wheel's notches come in lines, without gesture phases: a tilt notch right after three
    // vertical ones scrolls sideways as far as the tilt alone.
    let notches = |tilt_after_vertical: bool| {
        let mut h = harness();
        let before = scroll(&h);
        let deltas: &[Vec2] =
            if tilt_after_vertical { &[vec2(0.0, -3.0), vec2(0.0, -3.0), vec2(0.0, -3.0), vec2(-3.0, 0.0)] } else { &[vec2(-3.0, 0.0)] };
        for &delta in deltas {
            h.input_mut().events.push(Event::MouseWheel { unit: MouseWheelUnit::Line, delta, phase: TouchPhase::Move, modifiers: Modifiers::NONE });
            h.run_steps(2);
        }
        h.run_steps(30);
        scroll(&h) - before
    };
    let (alone, after_vertical) = (notches(false), notches(true));
    assert!(alone.x > 100.0 && alone.y == 0.0, "a tilt notch alone: {alone:?}");
    assert!(after_vertical.y > 100.0, "the vertical notches scroll: {after_vertical:?}");
    assert_eq!(after_vertical.x, alone.x, "the tilt scrolls as far after them: {after_vertical:?} vs {alone:?}");
}

#[test]
fn a_mouse_wheel_reporting_points_is_not_held_to_an_axis() {
    // On the web a mouse wheel reports points with no gesture phases, like a trackpad there;
    // without phases nothing is steered, so a tilt notch right after a vertical one scrolls
    // sideways as far as the tilt alone, and the vertical notch isn't cut by it.
    let notches = |tilt: bool| {
        let mut h = harness();
        let before = scroll(&h);
        let deltas: &[Vec2] = if tilt { &[vec2(0.0, -100.0), vec2(-100.0, 0.0)] } else { &[vec2(0.0, -100.0)] };
        for &delta in deltas {
            h.input_mut().events.push(Event::MouseWheel { unit: MouseWheelUnit::Point, delta, phase: TouchPhase::Move, modifiers: Modifiers::NONE });
            h.run_steps(2);
        }
        h.run_steps(60);
        scroll(&h) - before
    };
    let (alone, both) = (notches(false), notches(true));
    assert!(alone.y > 100.0 && alone.x == 0.0, "a vertical notch alone: {alone:?}");
    assert!(both.x > 100.0, "the tilt scrolls sideways: {both:?}");
    assert_eq!(both.y, alone.y, "the vertical notch is not cut by the tilt: {both:?} vs {alone:?}");
}

#[test]
fn shift_carried_only_by_the_wheel_events_still_holds_the_scroll() {
    // On the web without keyboard focus no modifier change reaches egui, but the wheel events
    // carry Shift and egui turns them horizontal from that; without phases the rail leaves the
    // scroll alone, so the swipe scrolls right.
    let mut h = harness();
    let before = scroll(&h);
    for _ in 0..8 {
        h.input_mut().events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, -12.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::SHIFT,
        });
        h.step();
    }
    let after = scroll(&h);
    assert!(after.x > before.x + 50.0, "scrolls right: {before:?} -> {after:?}");
    assert_eq!(after.y, before.y, "and not down: {before:?} -> {after:?}");
}

#[test]
fn a_gestures_end_leaves_nothing_of_it() {
    // The trackpad reported the gesture's end: a diagonal movement without phases right after it
    // (another device) moves both ways, not along the gesture's axis.
    let mut h = harness();
    swipe(&mut h, vec2(-1.0, -12.0), 6);
    let before = scroll(&h);
    for _ in 0..8 {
        wheel(&mut h, TouchPhase::Move, vec2(-8.0, -8.0));
    }
    let after = scroll(&h);
    assert!(after.x > before.x + 30.0 && after.y > before.y + 30.0, "both ways: {before:?} -> {after:?}");
}

#[test]
fn a_gesture_right_after_a_wheel_notch_steers_only_its_own_movement() {
    // A mouse and a trackpad side by side: a tilt notch in lines, then a vertical gesture starting
    // the next frame. What egui is still smoothing of the notch isn't the gesture's and goes
    // through whole, as far as the notch alone; the gesture itself is held to its axis.
    let notch_then = |gesture: bool| {
        let mut h = harness();
        let before = scroll(&h);
        h.input_mut().events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(-3.0, 0.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.step();
        if gesture {
            swipe(&mut h, vec2(0.0, -12.0), 10);
        }
        h.run_steps(60);
        scroll(&h) - before
    };
    let (alone, with_gesture) = (notch_then(false), notch_then(true));
    assert!(alone.x > 100.0 && alone.y == 0.0, "the notch alone: {alone:?}");
    // egui itself drops what little of the notch is still being smoothed when the gesture ends
    // (about 1 % after ten frames); the rail passes all of it until then.
    assert!(with_gesture.x >= alone.x * 0.98, "the notch scrolls beside the gesture: {with_gesture:?} vs {alone:?}");
    assert!(with_gesture.y > 100.0, "and the gesture scrolls down: {with_gesture:?}");
}

#[test]
fn the_axis_is_decided_after_eight_points_not_on_the_first() {
    // A gesture's first point can lean the wrong way; the axis is decided once the gesture has
    // gone the deciding distance, from everything so far, and then stays decided: two points
    // down after a sideways lean settle it, and a wander the other way afterwards is held.
    let mut h = harness();
    let before = scroll(&h);
    wheel(&mut h, TouchPhase::Start, Vec2::ZERO);
    wheel(&mut h, TouchPhase::Move, vec2(-2.0, -1.0));
    for _ in 0..2 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.0, -12.0));
    }
    let decided = scroll(&h);
    for _ in 0..6 {
        wheel(&mut h, TouchPhase::Move, vec2(-12.0, -1.0));
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(decided.y > before.y + 15.0, "the gesture goes down despite its first point: {before:?} -> {decided:?}");
    assert!(decided.x <= before.x + 3.0, "only the first point's lean passed: {before:?} -> {decided:?}");
    assert_eq!(after.x, decided.x, "held once decided: {decided:?} -> {after:?}");
    assert!(after.y > decided.y, "and still scrolling down: {decided:?} -> {after:?}");
}

#[test]
fn a_gesture_three_times_further_along_than_across_is_held() {
    // Twice as far along as across is the boundary; a gesture at three to one (about 18° off) is
    // held, not free, and so is one at two and a half to one (about 22°). One at one and three
    // quarters to one (about 30°) is free.
    let held = |delta: Vec2| {
        let mut h = harness();
        let before = scroll(&h);
        swipe(&mut h, delta, 10);
        let after = scroll(&h);
        assert!(after.y > before.y + 100.0, "scrolled down: {before:?} -> {after:?}");
        after.x == before.x
    };
    assert!(held(vec2(-4.0, -12.0)), "held to its axis at three to one");
    assert!(held(vec2(-4.8, -12.0)), "held to its axis at two and a half to one");
    assert!(!held(vec2(-7.0, -12.0)), "free at one and three quarters to one");
}

#[test]
fn the_deciding_distance_is_eight_points() {
    // Five points sideways, then five down: at seven points of travel nothing is decided and a
    // gesture leaning neither way moves both ways; at eleven, having gone twice as far down as
    // across, it is vertical, and a sideways point after that is held. (A shorter deciding
    // distance would have called it horizontal on the first point; a longer one would still let
    // the last point through.)
    let mut h = harness();
    let start = scroll(&h);
    wheel(&mut h, TouchPhase::Start, Vec2::ZERO);
    wheel(&mut h, TouchPhase::Move, vec2(-5.0, 0.0));
    let leaned = scroll(&h);
    wheel(&mut h, TouchPhase::Move, vec2(0.0, -5.0));
    let squared = scroll(&h);
    wheel(&mut h, TouchPhase::Move, vec2(0.0, -5.0));
    let decided = scroll(&h);
    wheel(&mut h, TouchPhase::Move, vec2(-5.0, 0.0));
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(leaned.x > start.x, "the first point leans sideways: {start:?} -> {leaned:?}");
    assert!(
        (squared.y - leaned.y - (leaned.x - start.x)).abs() < 0.01,
        "undecided at seven points, the gesture moves down as far: {leaned:?} -> {squared:?}"
    );
    assert!(decided.y > squared.y, "and on: {squared:?} -> {decided:?}");
    assert_eq!(after.x, decided.x, "vertical once decided at eleven points: {decided:?} -> {after:?}");
}

#[test]
fn alt_holds_a_gesture_vertical_and_the_rail_lets_it() {
    // Alt is egui's vertical scroll modifier: it turns a gesture's whole movement vertical, drift
    // included. Like Shift, it holds the gesture, so the rail passes egui's turned scroll as it
    // is: the same distance as a plain gesture of that total movement.
    let with_alt = {
        let mut h = harness();
        let before = scroll(&h);
        h.input_mut().events.push(Event::ModifiersChanged(Modifiers::ALT));
        h.input_mut().events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: Vec2::ZERO,
            phase: TouchPhase::Start,
            modifiers: Modifiers::ALT,
        });
        h.step();
        for _ in 0..8 {
            h.input_mut().events.push(Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: vec2(-12.0, -1.5),
                phase: TouchPhase::Move,
                modifiers: Modifiers::ALT,
            });
            h.step();
        }
        wheel(&mut h, TouchPhase::End, Vec2::ZERO);
        scroll(&h) - before
    };
    let plain = {
        let mut h = harness();
        let before = scroll(&h);
        swipe(&mut h, vec2(0.0, -13.5), 8);
        scroll(&h) - before
    };
    assert!(with_alt.y > 50.0 && with_alt.x == 0.0, "Alt+gesture scrolls vertically: {with_alt:?}");
    assert_eq!(with_alt, plain, "as far as egui turned it, drift included: {with_alt:?} vs {plain:?}");
}

#[test]
fn a_held_gesture_ending_in_the_frame_the_next_starts_does_not_hold_the_next() {
    // The end of a Shift gesture and the start of a plain one can share a frame: the modifier on
    // the earlier gesture's moves must not hold the new gesture, which is railed on its own.
    let mut h = harness();
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::SHIFT));
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: Vec2::ZERO,
        phase: TouchPhase::Start,
        modifiers: Modifiers::SHIFT,
    });
    h.step();
    for _ in 0..4 {
        h.input_mut().events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, -12.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::SHIFT,
        });
        h.step();
    }
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
    // One frame: the Shift gesture's last move and end, then the next gesture's start and move.
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, -12.0),
        phase: TouchPhase::Move,
        modifiers: Modifiers::SHIFT,
    });
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: Vec2::ZERO,
        phase: TouchPhase::End,
        modifiers: Modifiers::SHIFT,
    });
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: Vec2::ZERO,
        phase: TouchPhase::Start,
        modifiers: Modifiers::NONE,
    });
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(-1.5, -12.0),
        phase: TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    h.step();
    let before = scroll(&h);
    for _ in 0..8 {
        wheel(&mut h, TouchPhase::Move, vec2(-1.5, -12.0));
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(after.y > before.y + 50.0, "the new gesture scrolls down: {before:?} -> {after:?}");
    assert_eq!(after.x, before.x, "and is held to its axis, not freed by the old gesture's Shift: {before:?} -> {after:?}");
}

#[test]
fn shift_and_alt_together_pass_the_scroll_as_it_was() {
    // With both of egui's axis modifiers held egui turns nothing, and the grid's own Shift
    // handling only turns a scroll whose sideways part is zero. The rail leaves a gesture under
    // either modifier alone, so this one scrolls as it did before the rail: down, drift included.
    let mut h = harness();
    let before = scroll(&h);
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::SHIFT | Modifiers::ALT));
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: Vec2::ZERO,
        phase: TouchPhase::Start,
        modifiers: Modifiers::SHIFT | Modifiers::ALT,
    });
    h.step();
    for _ in 0..8 {
        h.input_mut().events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(-1.5, -12.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::SHIFT | Modifiers::ALT,
        });
        h.step();
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
    let after = scroll(&h);
    assert!(after.y > before.y + 50.0 && after.x > before.x, "scrolls down with its drift, as before the rail: {before:?} -> {after:?}");
}

#[test]
fn shift_on_a_wheel_notch_in_lines_holds_the_gesture_it_lands_in() {
    // egui takes a gesture's modifiers from every wheel event while it lasts, a mouse wheel's
    // lines included, and turns the gesture's movement from then on; the rail reads them there
    // too, so the turned scroll passes whole instead of being steered by the raw movement.
    let mut h = harness();
    swipe(&mut h, vec2(0.0, -12.0), 10);
    wheel(&mut h, TouchPhase::Start, Vec2::ZERO);
    for _ in 0..4 {
        wheel(&mut h, TouchPhase::Move, vec2(-12.0, -1.5));
    }
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::SHIFT));
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: vec2(0.0, -1.0),
        phase: TouchPhase::Move,
        modifiers: Modifiers::SHIFT,
    });
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
    let turned = scroll(&h);
    for _ in 0..8 {
        wheel(&mut h, TouchPhase::Move, vec2(-12.0, -1.5));
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(after.x > turned.x + 50.0, "keeps scrolling right: {turned:?} -> {after:?}");
    assert_eq!(after.y, turned.y, "and not up or down once egui turns the gesture: {turned:?} -> {after:?}");
}

#[test]
fn shift_on_a_move_without_movement_holds_the_gesture() {
    // egui takes a gesture's modifiers from every one of its wheel events, a move that carries no
    // movement included, and the grid doesn't scroll on such a frame at all; the rail must read
    // the modifier there too, or Shift pressed and released over a still finger would leave it
    // steering a gesture egui has turned horizontal, the drift's share scrolling up.
    let mut h = harness();
    swipe(&mut h, vec2(0.0, -12.0), 10);
    let before = scroll(&h);
    wheel(&mut h, TouchPhase::Start, Vec2::ZERO);
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::SHIFT));
    h.input_mut().events.push(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: Vec2::ZERO,
        phase: TouchPhase::Move,
        modifiers: Modifiers::SHIFT,
    });
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
    for _ in 0..10 {
        wheel(&mut h, TouchPhase::Move, vec2(-12.0, -1.5));
    }
    wheel(&mut h, TouchPhase::End, Vec2::ZERO);
    let after = scroll(&h);
    assert!(after.x > before.x + 50.0, "scrolls right: {before:?} -> {after:?}");
    assert_eq!(after.y, before.y, "and not up or down: {before:?} -> {after:?}");
}
