//! Shared operational limits for isolated workbook analysis, independent of UI or transport.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};

use crate::{EngineError, Result, Workbook};

/// Maximum estimated work units consumed by one trial sequence, including preparation.
pub const MAX_RUN_WORK: u64 = 100_000_000;
/// Estimated work units available per open workbook in a fixed sixty-second window.
pub const MAX_WINDOW_WORK: u64 = 300_000_000;
/// Analysis reports may not grow the active workbook beyond this stored-cell limit.
pub const MAX_REPORT_CELLS: usize = 2_000_000;
/// Includes source sheets, preventing unbounded small-report/chart accumulation.
pub const MAX_REPORT_SHEETS: usize = 128;
const WINDOW_MS: f64 = 60_000.0;

/// A cloneable cooperative cancellation signal. It never mutates the source workbook.
#[derive(Clone, Debug, Default)]
pub struct Cancellation(Arc<AtomicU8>);
impl Cancellation {
    /// Requests cancellation at the next analysis checkpoint, not an unsafe thread interruption.
    pub fn cancel(&self) -> bool {
        match self.0.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => true,
            Err(state) => state == 1,
        }
    }
    /// Whether cancellation has been requested by the UI, transport or embedding caller.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == 1
    }
    /// Converts a cancellation request into a normal atomic-analysis error.
    pub(crate) fn check(&self) -> Result<()> {
        if self.is_cancelled() { Err(EngineError::Other("Analysis cancelled; source cells were preserved.".into())) } else { Ok(()) }
    }

    /// Establishes the atomic publication boundary; later cancellation requests return false.
    pub(crate) fn begin_commit(&self) -> Result<()> {
        self.0
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| EngineError::Other("Analysis cancelled or already publishing; no partial report was created.".into()))
    }

    /// Marks publication complete without allowing a later request to claim it was cancelled.
    pub(crate) fn complete(&self) {
        self.0.store(3, Ordering::Release);
    }
}

#[derive(Debug, Default)]
struct Window {
    start_ms: Option<f64>,
    used: u64,
}

/// A budget shared by all what-if evaluators of an open workbook, including repeated requests.
/// Clones share counters; reopening a workbook creates a new budget, not a security sandbox.
#[derive(Clone, Debug, Default)]
pub struct AnalysisBudget(Arc<Mutex<Window>>);
impl AnalysisBudget {
    /// Charges estimated work against the workbook's fixed-window budget. Failed/cancelled work
    /// is not refunded, so retries cannot evade the shared limit. Clock rollback never refills it.
    pub(crate) fn spend(&self, units: u64) -> Result<()> {
        self.spend_at(units, gridcraft_core::date::unix_now_ms())
    }
    /// Applies a deterministic clock reading; absent/non-finite clocks conservatively do not refill.
    fn spend_at(&self, units: u64, now: Option<f64>) -> Result<()> {
        let mut window = self.0.lock().map_err(|_| EngineError::Other("Analysis budget is unavailable.".into()))?;
        refill(&mut window, now);
        let next = window.used.checked_add(units).filter(|n| *n <= MAX_WINDOW_WORK).ok_or_else(|| {
            EngineError::Other(
                "Shared analysis work limit reached for this workbook. Wait for the next 60-second window or reduce the model/iterations.".into(),
            )
        })?;
        window.used = next;
        Ok(())
    }
    /// Returns remaining estimated work after applying any elapsed fixed-window refill.
    pub fn remaining(&self) -> Result<u64> {
        let mut window = self.0.lock().map_err(|_| EngineError::Other("Analysis budget is unavailable.".into()))?;
        refill(&mut window, gridcraft_core::date::unix_now_ms());
        Ok(MAX_WINDOW_WORK.saturating_sub(window.used))
    }
}

/// Refills only after an entire forward-moving window; unknown/backward clocks retain charges.
fn refill(window: &mut Window, now: Option<f64>) {
    let Some(now) = now.filter(|n| n.is_finite()) else { return };
    match window.start_ms {
        Some(start) if now - start >= WINDOW_MS => {
            window.start_ms = Some(now);
            window.used = 0;
        }
        None => window.start_ms = Some(now),
        _ => {}
    }
}

/// A persistent run state: a caught #NUM cannot replenish work or erase the terminal error.
#[derive(Debug)]
struct RunState {
    budget: AnalysisBudget,
    spent: u64,
    limit: u64,
    failure: Option<String>,
}

/// One shared meter for trial bookkeeping, expression evaluation and native function kernels.
#[derive(Clone, Debug)]
pub(crate) struct Meter {
    pub cancellation: Cancellation,
    state: Arc<Mutex<RunState>>,
}
impl Meter {
    /// Starts a bounded run without resetting the open workbook's shared fixed-window bank.
    pub fn new(budget: AnalysisBudget) -> Self {
        Self { cancellation: Cancellation::default(), state: Arc::new(Mutex::new(RunState { budget, spent: 0, limit: MAX_RUN_WORK, failure: None })) }
    }
    /// Lets callers lower the run ceiling; terminal failures cannot be reset by changing limits.
    pub fn set_limit(&self, limit: u64) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| EngineError::Other("Analysis meter is unavailable.".into()))?;
        if limit == 0 || limit > state.limit || limit < state.spent {
            return Err(EngineError::Other(format!("Analysis work limit must be between 1 and {MAX_RUN_WORK} and cover work already spent.")));
        }
        state.limit = limit;
        Ok(())
    }
    /// Checks cancellation and the persistent failure independently of formula return values.
    pub fn check(&self) -> Result<()> {
        self.cancellation.check()?;
        let state = self.state.lock().map_err(|_| EngineError::Other("Analysis meter is unavailable.".into()))?;
        match &state.failure {
            Some(error) => Err(EngineError::Other(error.clone())),
            None => Ok(()),
        }
    }
    /// Reports consumed work without suppressing a poisoned-state error.
    pub fn spent(&self) -> Result<u64> {
        self.state.lock().map(|state| state.spent).map_err(|_| EngineError::Other("Analysis meter is unavailable.".into()))
    }
    /// Charges before a call/allocation and retains refusals even if a formula handles #NUM.
    pub fn charge(&self, units: u64) -> Result<()> {
        self.cancellation.check()?;
        let mut state = self.state.lock().map_err(|_| EngineError::Other("Analysis meter is unavailable.".into()))?;
        if let Some(error) = &state.failure {
            return Err(EngineError::Other(error.clone()));
        }
        let charged = match state.spent.checked_add(units).filter(|next| *next <= state.limit) {
            Some(next) => state.budget.spend(units).map(|_| next),
            None => Err(EngineError::Other("Analysis work limit exceeded. Reduce the model or iteration count.".into())),
        };
        match charged {
            Ok(next) => {
                state.spent = next;
                Ok(())
            }
            Err(error) => {
                state.failure = Some(error.to_string());
                Err(error)
            }
        }
    }
    /// Supplies lower-layer code a neutral callback, keeping engine policy/errors out of L1/L2.
    pub fn hook(&self) -> gridcraft_calc::WorkHook {
        let meter = self.clone();
        gridcraft_calc::WorkHook::new(move |units| meter.charge(units).is_ok())
    }
}

/// Checks report growth before trial allocation/publication, counting stored cells on all sheets.
/// Existing undo history remains governed by the engine's separate retained-byte/entry caps.
pub(crate) fn report_capacity(wb: &Workbook, additional_cells: usize) -> Result<()> {
    if wb.sheets.len() >= MAX_REPORT_SHEETS {
        return Err(EngineError::Other(format!(
            "Analysis report sheet limit reached ({MAX_REPORT_SHEETS} sheets including the source). Delete unneeded reports first."
        )));
    }
    let stored = wb.sheets.iter().fold(0usize, |n, sh| n.saturating_add(sh.cells.len()));
    if stored.saturating_add(additional_cells) > MAX_REPORT_CELLS {
        return Err(EngineError::Other(format!(
            "Analysis report storage limit exceeded ({MAX_REPORT_CELLS} stored cells including the source). Omit samples or delete unneeded reports."
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Repeated evaluators share a bank, including at rollback/unknown clock readings.
    #[test]
    fn shared_budget_refills_only_after_a_complete_forward_window() {
        let budget = AnalysisBudget::default();
        budget.spend_at(MAX_WINDOW_WORK, Some(1000.)).unwrap();
        assert!(budget.clone().spend_at(1, Some(1001.)).is_err());
        assert!(budget.spend_at(1, Some(0.)).is_err());
        assert!(budget.spend_at(1, None).is_err());
        assert!(budget.spend_at(1, Some(f64::NAN)).is_err());
        budget.spend_at(1, Some(61_000.)).unwrap();
    }
    /// Run limits cannot be raised and cancellation is observed before any extra charge.
    #[test]
    fn run_limits_and_cooperative_cancellation_are_enforced() {
        let meter = Meter::new(AnalysisBudget::default());
        meter.set_limit(10).unwrap();
        assert!(meter.set_limit(11).is_err());
        meter.charge(10).unwrap();
        assert!(meter.charge(1).is_err());
        assert!(meter.charge(0).is_err());
        assert!(meter.set_limit(MAX_RUN_WORK + 1).is_err());
        let next = Meter::new(AnalysisBudget::default());
        next.cancellation.clone().cancel();
        assert!(next.charge(1).is_err());
        assert_eq!(next.spent().unwrap(), 0);
    }

    /// Cancellation wins before publication; accepted publication rejects all later requests.
    #[test]
    fn cancellation_and_publication_have_one_atomic_boundary() {
        let cancelled = Cancellation::default();
        assert!(cancelled.cancel());
        assert!(cancelled.begin_commit().is_err());
        let publishing = Cancellation::default();
        publishing.begin_commit().unwrap();
        assert!(!publishing.cancel());
        assert!(!publishing.is_cancelled());
        publishing.complete();
        assert!(!publishing.cancel());
        assert!(publishing.check().is_ok());
    }
}
