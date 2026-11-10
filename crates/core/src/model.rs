//! What the audit reads from the tenancy, in the shape OCI returns it.

use serde::Deserialize;

use crate::date::Date;

/// Everything the checks look at — collected by [`crate::api::collect`], or built by hand in
/// tests. Each list that could not be read holds the error instead.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Tenancy name, e.g. `alexpua`.
    pub tenancy_name: String,
    /// Tenancy OCID.
    pub tenancy_id: String,
    /// Home region, e.g. `eu-amsterdam-1` — the only region where compute and storage are free.
    pub home_region: String,
    /// Every subscribed region.
    pub regions: Vec<String>,
    /// `PAYG`, ... from the Organizations API; `None` if it could not be read.
    pub payment_model: Option<String>,
    /// Instances in the home region, all compartments.
    pub instances: Listed<Instance>,
    /// Boot volumes in the home region.
    pub boot_volumes: Listed<Volume>,
    /// Block volumes in the home region.
    pub block_volumes: Listed<Volume>,
    /// Boot and block volume backups in the home region.
    pub backups: Listed<Backup>,
    /// Daily cost by service and SKU over the audited period.
    pub usage: Listed<UsageItem>,
    /// Budgets on the tenancy.
    pub budgets: Listed<Budget>,
    /// The day the audit runs (UTC).
    pub today: Date,
    /// First day of the audited period.
    pub since: Date,
}

/// A list read from OCI, or why it could not be.
pub type Listed<T> = Result<Vec<T>, String>;

/// A compute instance.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    /// OCID.
    pub id: String,
    /// Name in the console. `null` when the caller may only `inspect` instances.
    #[serde(default)]
    pub display_name: Option<String>,
    /// e.g. `VM.Standard.A1.Flex`.
    pub shape: String,
    /// `RUNNING`, `STOPPED`, `TERMINATED`, ...
    pub lifecycle_state: String,
    /// OCPUs and memory.
    #[serde(default)]
    pub shape_config: Option<ShapeConfig>,
}

impl Instance {
    /// The console name, or the end of the OCID when OCI hides the name.
    pub fn name(&self) -> String {
        name_or_ocid(self.display_name.as_deref(), &self.id, "instance")
    }

    /// Counts against limits: anything not terminated (or terminating).
    pub fn exists(&self) -> bool {
        !matches!(self.lifecycle_state.as_str(), "TERMINATED" | "TERMINATING")
    }

    /// Billed for compute right now.
    pub fn running(&self) -> bool {
        matches!(
            self.lifecycle_state.as_str(),
            "RUNNING" | "STARTING" | "PROVISIONING"
        )
    }

    /// OCPUs (0 if unknown).
    pub fn ocpus(&self) -> f64 {
        self.shape_config
            .as_ref()
            .and_then(|c| c.ocpus)
            .unwrap_or(0.0)
    }

    /// Memory in GB (0 if unknown).
    pub fn memory_gb(&self) -> f64 {
        self.shape_config
            .as_ref()
            .and_then(|c| c.memory_in_gbs)
            .unwrap_or(0.0)
    }
}

/// OCPUs and memory of an instance.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeConfig {
    /// OCPUs.
    pub ocpus: Option<f64>,
    /// Memory, GB. OCI spells it `memoryInGBs`, not camelCase.
    #[serde(rename = "memoryInGBs")]
    pub memory_in_gbs: Option<f64>,
}

/// A boot or block volume.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    /// OCID.
    pub id: String,
    /// Name in the console (`null` with `inspect`-only access).
    #[serde(default)]
    pub display_name: Option<String>,
    /// Size, GB. OCI spells it `sizeInGBs`, not camelCase.
    #[serde(default, rename = "sizeInGBs")]
    pub size_in_gbs: Option<f64>,
    /// `AVAILABLE`, `TERMINATED`, ...
    pub lifecycle_state: String,
}

impl Volume {
    /// Counts against the 200 GB: anything not terminated (or terminating).
    pub fn exists(&self) -> bool {
        !matches!(self.lifecycle_state.as_str(), "TERMINATED" | "TERMINATING")
    }
}

/// A boot or block volume backup.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    /// OCID.
    pub id: String,
    /// Name in the console (`null` with `inspect`-only access).
    #[serde(default)]
    pub display_name: Option<String>,
    /// `AVAILABLE`, `CREATING`, `TERMINATED`, ...
    pub lifecycle_state: String,
}

impl Backup {
    /// Counts against the 5 backups.
    pub fn exists(&self) -> bool {
        !matches!(
            self.lifecycle_state.as_str(),
            "TERMINATED" | "TERMINATING" | "FAULTY"
        )
    }
}

/// One row of `RequestSummarizedUsages`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageItem {
    /// Start of the day (or month) the row covers.
    pub time_usage_started: String,
    /// e.g. `Compute`.
    #[serde(default)]
    pub service: Option<String>,
    /// e.g. `Standard - A1 - Memory`.
    #[serde(default)]
    pub sku_name: Option<String>,
    /// Cost.
    #[serde(default)]
    pub computed_amount: Option<f64>,
    /// Quantity in `unit`.
    #[serde(default)]
    pub computed_quantity: Option<f64>,
    /// e.g. `EUR`.
    #[serde(default)]
    pub currency: Option<String>,
    /// e.g. `GB`, `OCPU Hours`.
    #[serde(default)]
    pub unit: Option<String>,
}

impl UsageItem {
    /// The day the row starts.
    pub fn date(&self) -> Option<Date> {
        Date::parse(&self.time_usage_started)
    }

    /// Cost, 0 if missing.
    pub fn amount(&self) -> f64 {
        self.computed_amount.unwrap_or(0.0)
    }
}

/// A budget.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    /// Name in the console.
    pub display_name: String,
    /// Budget amount.
    #[serde(default)]
    pub amount: Option<f64>,
    /// Alert rules on it — a budget without one emails nobody.
    #[serde(default)]
    pub alert_rule_count: Option<u32>,
}

/// `name`, or `instance …lprq` from the OCID when OCI did not return a name.
fn name_or_ocid(name: Option<&str>, id: &str, kind: &str) -> String {
    match name {
        Some(n) if !n.is_empty() => n.to_owned(),
        _ => {
            let tail: String = id
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("{kind} …{tail}")
        }
    }
}
