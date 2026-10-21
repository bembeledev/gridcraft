//! The seam a collaboration layer plugs into.
//!
//! The engine knows that something may want to hear about a command it just
//! ran. It deliberately knows nothing about what that something is. A relay, a
//! log file and a test double are all the same shape from here.
//!
//! This is the only thing the engine gains for shared editing. Ordering,
//! locking, transport and credentials all live outside it.

use serde_json::Value as Json;

use crate::cmd::find_command;

/// Receives commands that originated on this machine.
///
/// Called after a journaled command has succeeded, so an implementation sees
/// exactly the `(id, params)` pairs that changed the workbook. Commands that
/// only move the view or open a dialog are not journaled and never arrive
/// here.
///
/// A command applied through [`crate::Session::apply_remote`] is not reported,
/// which is what stops two peers echoing one edit back and forth forever.
pub trait CollabSink: Send + Sync {
    /// Forward one local command. Must not block: it runs inside the command.
    fn on_local(&self, id: &str, params: &Json);
}

/// Whether a command may leave this machine, or be applied if it arrives.
///
/// Journaled is necessary but not enough. `file.open`, `file.save` and
/// `file.new` are journaled so a macro can replay them, and so is `edit.undo`.
/// None of those is a shared edit: a peer must not open a path from another
/// machine, and undo is local to this peer's own history.
pub fn shareable(id: &str) -> bool {
    let Some(spec) = find_command(id) else {
        return false;
    };
    if !spec.journal {
        return false;
    }
    if matches!(id, "edit.undo" | "edit.redo") {
        return false;
    }
    let family = id.split('.').next().unwrap_or(id);
    !matches!(family, "file" | "print" | "view" | "inspect")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gridcraft_core::{CellRef, Value};
    use serde_json::json;

    use super::*;
    use crate::Session;

    #[derive(Default)]
    struct Heard(Mutex<Vec<String>>);

    impl CollabSink for Heard {
        fn on_local(&self, id: &str, _params: &Json) {
            self.0.lock().unwrap().push(id.to_owned());
        }
    }

    fn book() -> Session {
        let mut s = Session::new();
        s.new_workbook();
        s
    }

    fn cell(s: &Session, a: &str) -> Value {
        s.doc().unwrap().wb.active().unwrap().value(CellRef::parse(a).unwrap())
    }

    fn cell_of(s: &Session, doc: usize, a: &str) -> Value {
        s.documents()[doc].wb.active().unwrap().value(CellRef::parse(a).unwrap())
    }

    #[test]
    fn a_cell_edit_is_reported_and_a_file_command_is_not() {
        let mut s = book();
        let heard = Arc::new(Heard::default());
        s.collab = Some(heard.clone());
        s.execute("cell.set", json!({"cell": "A1", "input": "x"})).unwrap();
        s.execute("file.new", json!({})).unwrap();
        let heard = heard.0.lock().unwrap();
        assert_eq!(&*heard, &["cell.set"], "only the workbook mutation is shared");
    }

    #[test]
    fn a_remote_file_command_is_refused_and_does_not_run() {
        let mut s = book();
        let err = s.apply_remote("file.open", json!({"path": "C:/nope.xlsx"})).unwrap_err().to_string();
        assert!(err.contains("not a shared edit"), "{err}");
        assert_eq!(s.documents().len(), 1, "no workbook was opened");
    }

    #[test]
    fn a_remote_undo_is_refused() {
        let mut s = book();
        s.execute("cell.set", json!({"cell": "A1", "input": "x"})).unwrap();
        let err = s.apply_remote("edit.undo", json!({})).unwrap_err().to_string();
        assert!(err.contains("not a shared edit"), "{err}");
        assert_eq!(cell(&s, "A1"), Value::text("x"), "the local cell was left alone");
    }

    #[test]
    fn a_remote_edit_is_not_on_the_local_undo_stack() {
        let mut s = book();
        s.execute("cell.set", json!({"cell": "A1", "input": "mine"})).unwrap();
        s.apply_remote("cell.set", json!({"cell": "B1", "input": "theirs"})).unwrap();
        assert_eq!(cell(&s, "B1"), Value::text("theirs"));
        assert!(s.active().unwrap().undo.is_empty(), "a peer's edit is not ours to undo");
        assert!(s.execute("edit.undo", json!({})).is_err(), "nothing local left to undo");
        assert_eq!(cell(&s, "B1"), Value::text("theirs"));
        assert_eq!(cell(&s, "A1"), Value::text("mine"), "clearing history does not revert earlier local edits");
    }

    #[test]
    fn a_local_undo_is_not_sent_to_peers() {
        let mut s = book();
        let heard = Arc::new(Heard::default());
        s.collab = Some(heard.clone());
        s.execute("cell.set", json!({"cell": "A1", "input": "x"})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        let heard = heard.0.lock().unwrap();
        assert_eq!(&*heard, &["cell.set"]);
    }

    #[test]
    fn only_the_shared_workbook_is_forwarded_and_remote_edits_land_there() {
        let mut s = Session::new();
        s.new_workbook();
        let uid_a = s.active().unwrap().uid;
        s.new_workbook();
        let heard = Arc::new(Heard::default());
        s.collab = Some(heard.clone());
        s.collab_uid = Some(uid_a);
        s.execute("cell.set", json!({"cell": "A1", "input": "on-b"})).unwrap();
        assert!(heard.0.lock().unwrap().is_empty(), "the other workbook is not in the room");
        s.set_active(0);
        s.execute("cell.set", json!({"cell": "A1", "input": "on-a"})).unwrap();
        assert_eq!(&*heard.0.lock().unwrap(), &["cell.set"]);
        s.set_active(1);
        s.apply_remote("cell.set", json!({"cell": "B1", "input": "remote"})).unwrap();
        assert_eq!(s.active_index(), 1, "the window the user is in is put back");
        assert_eq!(cell_of(&s, 0, "B1"), Value::text("remote"));
        assert!(cell_of(&s, 1, "B1").is_empty(), "the other workbook was not written");
    }
}
