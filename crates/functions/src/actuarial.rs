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

/// Apply ACT defaults to omitted slots and empty literals, retaining ordinary blank-reference coercion.
pub(super) fn optional_num(args: &[crate::Arg], index: usize, default: f64) -> crate::util::R<f64> {
    if crate::util::has(args, index) { crate::util::num(args, index) } else { Ok(default) }
}

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
