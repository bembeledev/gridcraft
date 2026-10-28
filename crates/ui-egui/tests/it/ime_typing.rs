//! Typing with an input method straight into a cell (fcitx5-unikey's Vietnamese, as XIM's
//! on-the-spot style delivers it): the word being composed opens the cell's editor and shows in
//! the cell, and the finished word goes in as typed text (before, the composition stayed in a box
//! of the input method's own until Space).

use egui::{Event, ImeEvent, Key, vec2};
use gridcraft_engine::Session;

use crate::common::{Harness, app, builder, harness_with};

fn harness() -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = harness_with(builder().with_size(vec2(1200.0, 800.0)), app(session), 0);
    for _ in 0..4 {
        step(&mut h);
    }
    h
}

/// One frame, with the app's queued events first, as eframe's `raw_input_hook` gives them.
fn step(h: &mut Harness) {
    if let Some(batch) = h.state_mut().synthetic.pop_front() {
        h.input_mut().events.extend(batch);
    }
    h.run_steps(1);
}

fn preedit(text: &str) -> Event {
    let n = text.chars().count();
    Event::Ime(ImeEvent::Preedit { text: text.into(), active_range_chars: Some(n..n) })
}

fn a1(h: &Harness) -> String {
    let s = &h.state().session;
    let d = s.active().unwrap();
    gridcraft_engine::display::edit_text(&d.wb, d.wb.active().unwrap(), gridcraft_engine::core::CellRef::new(0, 0))
}

#[test]
fn composing_straight_into_a_cell_shows_in_the_cell() {
    let mut h = harness();
    assert!(h.output().platform_output.ime.is_some(), "the input method is on at the active cell");
    // Telex "chaof" as unikey composes it, one keystroke a frame (letters the test fonts have).
    for word in ["c", "ch", "cha", "chao", "chào"] {
        h.event(preedit(word));
        step(&mut h);
        step(&mut h);
    }
    let shown = h.state().editor.as_ref().map(|e| e.text.clone());
    assert_eq!(shown.as_deref(), Some("chào"), "the word being composed is in the cell's editor");
    // Space finishes the word: the composition is cleared, the word committed, the space typed.
    h.event(preedit(""));
    h.event(Event::Ime(ImeEvent::Commit("chào".into())));
    step(&mut h);
    h.event(Event::Text(" ".into()));
    step(&mut h);
    h.key_press(Key::Enter);
    step(&mut h);
    step(&mut h);
    assert_eq!(a1(&h).trim_end(), "chào");
}

/// The first letter of a word opens the cell's editor and moves the focus there; egui interrupts
/// the input method on every focus move, and fcitx5 then dropped the letter it was composing
/// ("không" came out "hong"). The input method stays on and uninterrupted throughout.
#[test]
fn opening_the_editor_keeps_the_letter_being_composed() {
    let mut h = harness();
    h.event(preedit("k"));
    for frame in 0..4 {
        step(&mut h);
        let ime = h.output().platform_output.ime.clone();
        assert!(ime.is_some(), "frame {frame}: the input method stays on");
        assert!(!ime.unwrap().should_interrupt_composition, "frame {frame}: the composition of \"k\" is not interrupted");
    }
    assert_eq!(h.state().editor.as_ref().map(|e| e.text.as_str()), Some("k"));
    // The input method goes on with the same word.
    for word in ["kh", "kho", "khô", "khôn", "không"] {
        h.event(preedit(word));
        step(&mut h);
        assert!(!h.output().platform_output.ime.as_ref().unwrap().should_interrupt_composition);
    }
    h.event(preedit(""));
    h.event(Event::Ime(ImeEvent::Commit("không".into())));
    step(&mut h);
    h.key_press(Key::Enter);
    step(&mut h);
    step(&mut h);
    assert_eq!(a1(&h), "không");
}

/// An input method that sends only finished words (no composition) types them into the cell.
#[test]
fn a_finished_word_without_a_composition_is_typed_into_the_cell() {
    let mut h = harness();
    h.event(Event::Ime(ImeEvent::Commit("Đà".into())));
    step(&mut h);
    assert_eq!(h.state().editor.as_ref().map(|e| e.text.as_str()), Some("Đà"));
    h.key_press(Key::Enter);
    step(&mut h);
    assert_eq!(a1(&h), "Đà");
}
