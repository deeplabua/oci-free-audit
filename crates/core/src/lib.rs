//! `oci-free-audit-core` — reads an Oracle Cloud tenancy (never changes it) and checks it
//! against the Always Free limits and the actual bill.
//!
//! - [`api::collect`]: reads the tenancy into a [`Snapshot`]
//! - [`checks::audit`]: a [`Snapshot`] in, a [`Report`] of ✓ / ⚠ / ✗ findings out
//! - [`limits`]: Oracle's numbers, with the date they were checked

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod api;
pub mod checks;
pub mod date;
pub mod limits;
pub mod model;
pub mod report;

pub use checks::{Options, audit};
pub use date::Date;
pub use model::Snapshot;
pub use report::{Finding, Report, Section, Status};
