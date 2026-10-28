//! GridCraft actuarial extensions. Pure, deterministic worksheet functions (L1).
//!
//! `ACT.*` is a GridCraft extension, not an Excel-compatible function namespace.
//! See `docs/actuarial.md` for probability, timing and truncation conventions.

mod bootstrap;
mod decision;
mod extreme;
mod inference;
mod life;
mod loss;
mod markov;
mod reinsurance;
mod reserving;

use crate::FnSpec;

/// Collect all deterministic actuarial worksheet specifications without introducing an engine dependency.
pub(crate) fn specs() -> Vec<FnSpec> {
    let mut result = life::specs();
    result.extend(loss::specs());
    result.extend(markov::specs());
    result.extend(inference::specs());
    result.extend(reinsurance::specs());
    result.extend(extreme::specs());
    result.extend(decision::specs());
    result.extend(bootstrap::specs());
    result.extend(reserving::specs());
    result
}
