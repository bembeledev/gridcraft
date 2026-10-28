use super::*;
use std::sync::Arc;

/// Builds deterministic source data and canonical cross-sheet simulation parameters.
fn fixture() -> (Session, Json) {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell":"A1","input":"10"})).unwrap();
    s.execute("cell.set", json!({"cell":"B1","input":"=A1*2"})).unwrap();
    let p = json!({"inputs":[{"cell":"Sheet1!A1","min":0,"max":1}],"outputs":["Sheet1!B1"],"iterations":8,"samples":false,"seed":42});
    (s, p)
}

/// Completes a tiny job with explicit batches, avoiding assumptions about elapsed execution time.
fn ready(s: &mut Session, id: &Json) {
    for _ in 0..20 {
        let p = s.execute("analysis.step", json!({"job":id,"trials":2})).unwrap();
        if p["state"] == "ready" {
            return;
        }
    }
    panic!("fixture should finish in bounded steps");
}

/// Real trial progress can be cancelled without source/history changes; stale ids are harmless.
#[test]
fn cancellation_after_trials_is_atomic_and_ids_do_not_reuse() {
    let (mut s, p) = fixture();
    let original = s.doc().unwrap().wb.clone();
    let undo = s.doc().unwrap().undo.len();
    let journal = s.journal.len();
    let id = s.execute("analysis.start", json!({"params":p})).unwrap()["job"].clone();
    s.execute("analysis.step", json!({"job":id,"trials":1})).unwrap();
    assert_eq!(s.execute("analysis.step", json!({"job":id,"trials":1})).unwrap()["completed"], 1);
    assert!(s.execute("cell.set", json!({"cell":"A1","input":"99"})).is_err());
    assert!(s.execute("analysis.finish", json!({"job":id})).is_err());
    assert!(s.analysis_active());
    s.execute("analysis.cancel", json!({"job":id})).unwrap();
    assert!(Arc::ptr_eq(&original, &s.doc().unwrap().wb));
    assert_eq!(s.doc().unwrap().undo.len(), undo);
    assert_eq!(s.journal.len(), journal);
    let newer = s.execute("analysis.start", json!({"params":p})).unwrap()["job"].clone();
    assert_ne!(id, newer);
    assert!(s.execute("analysis.cancel", json!({"job":id})).is_err());
    assert!(s.analysis_active());
    s.execute("analysis.cancel", json!({"job":newer})).unwrap();
}

/// Publication matches synchronous statistics and creates exactly one replayable undo action.
#[test]
fn finish_matches_synchronous_results_and_undo() {
    let (mut sync, p) = fixture();
    let expected = sync.execute("data.monteCarlo", p.clone()).unwrap();
    let (mut s, _) = fixture();
    let original = s.doc().unwrap().wb.clone();
    let before = s.doc().unwrap().undo.len();
    let id = s.execute("analysis.start", json!({"params":p})).unwrap()["job"].clone();
    ready(&mut s, &id);
    assert!(Arc::ptr_eq(&original, &s.doc().unwrap().wb));
    let actual = s.execute("analysis.finish", json!({"job":id})).unwrap();
    assert_eq!(actual["outputs"], expected["outputs"]);
    assert_eq!(actual["tornado"], expected["tornado"]);
    assert_eq!(s.doc().unwrap().undo.len(), before + 1);
    assert_eq!(s.journal.last().unwrap().0, "data.monteCarlo");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheets.len(), 1);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheets.len(), 2);
}

/// External cancellation and edits through an embedding API cannot be silently overwritten.
#[test]
fn cancellation_handles_and_source_conflicts_are_checked() {
    let (mut s, p) = fixture();
    let id = s.execute("analysis.start", json!({"params":p})).unwrap()["job"].clone();
    assert!(s.analysis_cancellation().unwrap().cancel());
    assert!(s.execute("analysis.step", json!({"job":id})).is_err());
    assert!(!s.analysis_active());
    let id = s.execute("analysis.start", json!({"params":p})).unwrap()["job"].clone();
    let mut changed = (*s.doc().unwrap().wb).clone();
    changed.sheet_mut(0).unwrap().set_value(CellRef::new(0, 0), gridcraft_core::Value::Number(99.));
    s.doc_mut().unwrap().wb = Arc::new(changed);
    assert!(s.execute("analysis.step", json!({"job":id})).is_err());
    assert_eq!(s.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::new(0, 0)), gridcraft_core::Value::Number(99.));
}

/// Planning rejects excess work; repeated small reports cannot evade the workbook growth cap.
#[test]
fn work_and_cumulative_report_limits_are_atomic() {
    let (mut s, mut p) = fixture();
    p["maxWork"] = json!(1);
    assert!(s.execute("data.monteCarlo", p.clone()).is_err());
    assert_eq!(s.doc().unwrap().wb.sheets.len(), 1);
    p.as_object_mut().unwrap().remove("maxWork");
    p["iterations"] = json!(2);
    for _ in 1..crate::workload::MAX_REPORT_SHEETS {
        s.execute("data.monteCarlo", p.clone()).unwrap();
    }
    let before = s.doc().unwrap().wb.clone();
    assert!(s.execute("data.monteCarlo", p).is_err());
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().wb));
    assert!(s.doc().unwrap().undo.len() <= 100);
}

/// Volatile trials and unavailable external references never leak trial state into the source.
#[test]
fn volatile_and_external_formula_trials_do_not_leak() {
    let (mut s, p) = fixture();
    s.execute("cell.set", json!({"cell":"B1","input":"=A1+RAND()"})).unwrap();
    let original = s.doc().unwrap().wb.clone();
    let id = s.execute("analysis.start", json!({"params":p})).unwrap()["job"].clone();
    s.execute("analysis.step", json!({"job":id})).unwrap();
    s.execute("analysis.step", json!({"job":id,"trials":1})).unwrap();
    s.execute("analysis.cancel", json!({"job":id})).unwrap();
    assert!(Arc::ptr_eq(&original, &s.doc().unwrap().wb));
    s.execute("cell.set", json!({"cell":"B1","input":"='[Unavailable.xlsx]Sheet1'!A1"})).unwrap();
    let original = s.doc().unwrap().wb.clone();
    let id = s.execute("analysis.start", json!({"params":p})).unwrap()["job"].clone();
    s.execute("analysis.step", json!({"job":id})).unwrap();
    assert!(s.execute("analysis.step", json!({"job":id})).is_err());
    assert!(Arc::ptr_eq(&original, &s.doc().unwrap().wb));
}

/// A latched parent is rejected before handlers can retain new runtime state.
#[test]
fn latched_parent_cannot_start_or_retain_a_job() {
    let (mut s, params) = fixture();
    let hook = gridcraft_calc::WorkHook::new(|_| false);
    let _scope = hook.enter().unwrap();
    assert!(!gridcraft_calc::WorkHook::allowed());
    assert!(s.execute("analysis.start", json!({"params":params})).is_err());
    assert!(!s.analysis_active());
}

/// A policy may refuse only after preparation; failed starts must still discard their new job.
#[test]
fn post_handler_refusal_does_not_retain_a_new_job() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let (mut s, params) = fixture();
    let calls = Arc::new(AtomicU64::new(0));
    let seen = calls.clone();
    let hook = gridcraft_calc::WorkHook::new(move |_| seen.fetch_add(1, Ordering::Relaxed) == 0);
    let _scope = hook.enter().unwrap();
    assert!(s.execute("analysis.start", json!({"params":params})).is_err());
    assert!(!s.analysis_active());
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

/// Direct recalc handlers bypass Ctx::commit, so terminal work errors need the global barrier.
#[test]
fn direct_recalculation_failure_restores_source_history_revision_and_journal() {
    let (mut s, _) = fixture();
    s.execute("cell.set", json!({"cell":"D1","input":"99"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    let original = s.doc().unwrap().wb.clone();
    let selection = s.doc().unwrap().selection.clone();
    let revision = s.doc().unwrap().revision;
    let undo = s.doc().unwrap().undo.iter().map(|entry| (entry.label.clone(), entry.wb.clone())).collect::<Vec<_>>();
    let redo = s.doc().unwrap().redo.iter().map(|entry| (entry.label.clone(), entry.wb.clone())).collect::<Vec<_>>();
    let journal = s.journal.clone();
    let meter = crate::workload::Meter::new(s.doc().unwrap().analysis_budget.clone());
    meter.set_limit(1).unwrap();
    let _scope = meter.hook().enter().unwrap();
    assert!(s.execute("formulas.calculateNow", json!({})).is_err());
    let d = s.doc().unwrap();
    assert!(Arc::ptr_eq(&original, &d.wb));
    assert_eq!(d.selection, selection);
    assert_eq!(d.revision, revision);
    assert_eq!(d.undo.len(), undo.len());
    assert_eq!(d.redo.len(), redo.len());
    for (actual, (label, wb)) in d.undo.iter().zip(&undo).chain(d.redo.iter().zip(&redo)) {
        assert_eq!(&actual.label, label);
        assert!(Arc::ptr_eq(&actual.wb, wb));
    }
    assert_eq!(s.journal, journal);
}

/// Nested calls inherit one run meter and cannot hide refusal or lose a trimmed journal prefix.
#[test]
fn nested_handlers_share_work_and_restore_even_a_trimmed_journal() {
    /// Models a native kernel whose caller ignores its refusal after a nested edit.
    fn child(s: &mut Session, _: &Json) -> Result<Json> {
        s.execute("cell.set", json!({"cell":"C1","input":"99"}))?;
        let _ = gridcraft_calc::WorkHook::current().unwrap().charge(60);
        Ok(json!(true))
    }
    /// Intentionally absorbs a child error to test the persistent logical-operation latch.
    fn parent(s: &mut Session, _: &Json) -> Result<Json> {
        let _ = s.execute("test.work.child", json!({}));
        let _ = s.execute("test.work.child", json!({}));
        Ok(json!(true))
    }
    crate::register_test_command("test.work.child", child);
    crate::register_test_command("test.work.parent", parent);
    let (mut s, _) = fixture();
    s.journal = vec![("old".into(), json!({"kept":true})); 10_000];
    let original = s.doc().unwrap().wb.clone();
    let undo = s.doc().unwrap().undo.len();
    let revision = s.doc().unwrap().revision;
    let journal = s.journal.clone();
    let meter = crate::workload::Meter::new(s.doc().unwrap().analysis_budget.clone());
    meter.set_limit(100).unwrap();
    let _scope = meter.hook().enter().unwrap();
    assert!(s.execute("test.work.parent", json!({})).is_err());
    assert_eq!(meter.spent().unwrap(), 60);
    assert!(Arc::ptr_eq(&original, &s.doc().unwrap().wb));
    assert_eq!(s.doc().unwrap().undo.len(), undo);
    assert_eq!(s.doc().unwrap().revision, revision);
    assert_eq!(s.journal, journal);
}
