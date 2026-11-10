//! The audit result: sections of findings, each ✓ / ⚠ / ✗.

use serde::Serialize;

use crate::date::Date;

/// How a finding stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Free, within limits.
    Ok,
    /// A risk, or something that could not be checked.
    Warn,
    /// Charged, or over a limit.
    Fail,
}

impl Status {
    /// `✓`, `⚠` or `✗`.
    pub fn symbol(self) -> &'static str {
        match self {
            Status::Ok => "✓",
            Status::Warn => "⚠",
            Status::Fail => "✗",
        }
    }
}

/// One line of the report.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// ✓ / ⚠ / ✗.
    pub status: Status,
    /// What was checked, e.g. `A1.Flex total`.
    pub label: String,
    /// The answer, e.g. `4 OCPU / 24 GB, limit 2 OCPU / 12 GB`.
    pub detail: String,
    /// Specifics: instance names, cost by service, a hint.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
}

impl Finding {
    /// A finding without items.
    pub fn new(status: Status, label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            status,
            label: label.into(),
            detail: detail.into(),
            items: Vec::new(),
        }
    }

    /// Add specifics.
    pub fn with_items(mut self, items: Vec<String>) -> Self {
        self.items = items;
        self
    }

    /// "Could not check" — a warning: the audit cannot vouch for it.
    pub fn unknown(label: impl Into<String>, error: &str) -> Self {
        Self::new(Status::Warn, label, format!("could not check: {error}"))
    }
}

/// A group of findings, e.g. `COST`.
#[derive(Debug, Clone, Serialize)]
pub struct Section {
    /// e.g. `COST`.
    pub title: String,
    /// Its lines.
    pub findings: Vec<Finding>,
}

/// The whole result.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Tenancy name.
    pub tenancy: String,
    /// Tenancy OCID.
    pub tenancy_id: String,
    /// Home region.
    pub home_region: String,
    /// `PAYG`, ... if known.
    pub payment_model: Option<String>,
    /// The free trial's last day, while it is not over.
    pub trial_ends: Option<Date>,
    /// When the Always Free numbers were last checked.
    pub limits_as_of: &'static str,
    /// First day of the audited period.
    pub since: Date,
    /// The day of the audit (UTC).
    pub today: Date,
    /// The last day the Usage API has cost data for.
    pub cost_data_up_to: Option<Date>,
    /// The findings.
    pub sections: Vec<Section>,
    /// Worst status of all findings.
    pub result: Status,
}

impl Report {
    /// `0` all free, `1` warnings, `2` charges or limits exceeded.
    pub fn exit_code(&self) -> u8 {
        match self.result {
            Status::Ok => 0,
            Status::Warn => 1,
            Status::Fail => 2,
        }
    }

    /// Findings with this status.
    pub fn count(&self, status: Status) -> usize {
        self.sections
            .iter()
            .flat_map(|s| &s.findings)
            .filter(|f| f.status == status)
            .count()
    }
}
