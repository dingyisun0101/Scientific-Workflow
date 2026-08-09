//! Internal wall-clock formatting and monotonic-duration helpers.
//!
//! Scientific coordinates live in [`crate::system_state::SimulationTime`].
//! This module exists only for operational workflow timing: it obtains a UTC
//! timestamp for durable metadata and converts a process-local monotonic
//! [`std::time::Duration`] into an exact integer nanosecond count.
//!
//! Keeping these operations behind one private boundary prevents storage and
//! execution-scope code from choosing subtly different timestamp formats.

use std::time::Duration;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Returns the current UTC wall-clock time in canonical RFC 3339 form.
///
/// Nanosecond precision is requested by the `time` crate's well-known format;
/// insignificant trailing fractional digits may be omitted. The `Z` suffix is
/// guaranteed because the source value is explicitly UTC.
pub(crate) fn utc_now_rfc3339() -> Result<String, time::error::Format> {
    OffsetDateTime::now_utc().format(&Rfc3339)
}

/// Reports whether `value` is a syntactically valid UTC RFC 3339 timestamp.
pub(crate) fn is_utc_rfc3339(value: &str) -> bool {
    value.ends_with('Z')
        && OffsetDateTime::parse(value, &Rfc3339).is_ok_and(|time| time.offset().is_utc())
}

/// Converts a monotonic duration into an exact `u64` nanosecond count.
///
/// `None` is possible only for a duration longer than approximately 584 years;
/// callers turn that impossible-to-represent condition into their contextual
/// public error rather than truncating or saturating timing metadata.
pub(crate) fn duration_nanoseconds(duration: Duration) -> Option<u64> {
    u64::try_from(duration.as_nanos()).ok()
}
