//! What an extension tells the host about itself outside its extension
//! points: that it needs the user's attention, and why.
//!
//! An extension that has disabled itself because what it connects to is
//! not on the machine — the Tracker connector with neither `localsearch`
//! nor `tracker3` installed — reports it here, quietly: nothing is shown
//! where the extension would have worked, and the Extensions dialogue
//! marks the plugin with the attention dot and gives the reason (SPEC.md
//! *Attention*). The host and the plugins share this crate, so what a
//! plugin reports the host reads.
//!
//! # Worked example
//!
//! ```
//! use ext::attention;
//! use types::ExtId;
//!
//! const ID: ExtId = ExtId("search-example");
//! assert_eq!(attention::of(ID.0), None);
//! // The connector looked for its service and found none.
//! attention::report(ID, "Neither localsearch nor tracker3 was found");
//! assert_eq!(
//!     attention::of(ID.0).as_deref(),
//!     Some("Neither localsearch nor tracker3 was found")
//! );
//! // It is there after all.
//! attention::clear(ID);
//! assert_eq!(attention::of(ID.0), None);
//! ```

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use types::ExtId;

/// The reasons reported, by extension id.
static REPORTS: Mutex<BTreeMap<&'static str, String>> = Mutex::new(BTreeMap::new());

fn reports() -> MutexGuard<'static, BTreeMap<&'static str, String>> {
    // Plain data; a panic elsewhere leaves it usable.
    REPORTS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Says that the extension `id` needs the user's attention, with the
/// reason its card gives, a sentence in the user's language. A later
/// report replaces an earlier one.
pub fn report(id: ExtId, reason: impl Into<String>) {
    reports().insert(id.0, reason.into());
}

/// Takes the report of `id` back: it is in order again.
pub fn clear(id: ExtId) {
    reports().remove(id.0);
}

/// Why the extension with this id needs attention, if it said so.
#[must_use]
pub fn of(id: &str) -> Option<String> {
    reports().get(id).cloned()
}
