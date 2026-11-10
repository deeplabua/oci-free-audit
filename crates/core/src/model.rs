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
    /// Instances, every subscribed region, all compartments.
    pub instances: Listed<Instance>,
    /// Boot volumes, every region.
    pub boot_volumes: Listed<Volume>,
    /// Block volumes, every region.
    pub block_volumes: Listed<Volume>,
    /// Boot and block volume backups, every region.
    pub backups: Listed<Backup>,
    /// Object Storage buckets, every region.
    pub buckets: Listed<Bucket>,
    /// Autonomous Databases, every region.
    pub autonomous_dbs: Listed<Service>,
    /// MySQL HeatWave DB systems, every region.
    pub mysql: Listed<Service>,
    /// Load balancers, every region.
    pub load_balancers: Listed<LoadBalancer>,
    /// Network load balancers, every region.
    pub network_load_balancers: Listed<Service>,
    /// VCNs, every region.
    pub vcns: Listed<Service>,
    /// Vaults, every region.
    pub vaults: Listed<Service>,
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
    /// Region it lives in (set by `collect`).
    #[serde(skip)]
    pub region: String,
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
    /// Region it lives in (set by `collect`).
    #[serde(skip)]
    pub region: String,
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
    /// Region it lives in (set by `collect`).
    #[serde(skip)]
    pub region: String,
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

/// An Object Storage bucket.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    /// Bucket name.
    pub name: String,
    /// `Standard` or `Archive`.
    #[serde(default)]
    pub storage_tier: Option<String>,
    /// Bytes stored, from `GetBucket?fields=approximateSize`; `None` if that was refused.
    #[serde(default)]
    pub approximate_size: Option<f64>,
    /// Region it lives in (set by `collect`).
    #[serde(skip)]
    pub region: String,
}

/// A resource the checks only count or classify: Autonomous Database, MySQL DB system, NLB,
/// VCN, vault.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    /// OCID.
    pub id: String,
    /// Name in the console.
    #[serde(default)]
    pub display_name: Option<String>,
    /// `AVAILABLE`, `ACTIVE`, `STOPPED`, `TERMINATED`, `DELETED`, ...
    #[serde(default)]
    pub lifecycle_state: Option<String>,
    /// Autonomous Database: `true` for an Always Free one.
    #[serde(default)]
    pub is_free_tier: Option<bool>,
    /// MySQL: e.g. `MySQL.Free`.
    #[serde(default)]
    pub shape_name: Option<String>,
    /// Vault: `DEFAULT` (virtual) or `VIRTUAL_PRIVATE`.
    #[serde(default)]
    pub vault_type: Option<String>,
    /// Region it lives in (set by `collect`).
    #[serde(skip)]
    pub region: String,
}

impl Service {
    /// The console name, or the end of the OCID.
    pub fn name(&self, kind: &str) -> String {
        name_or_ocid(self.display_name.as_deref(), &self.id, kind)
    }

    /// Not deleted or being deleted.
    pub fn exists(&self) -> bool {
        !matches!(
            self.lifecycle_state.as_deref(),
            Some("TERMINATED" | "TERMINATING" | "DELETED" | "DELETING" | "PENDING_DELETION")
        )
    }
}

/// A load balancer.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadBalancer {
    /// OCID.
    pub id: String,
    /// Name in the console.
    #[serde(default)]
    pub display_name: Option<String>,
    /// `flexible`, `10Mbps-Micro`, `100Mbps`, ...
    #[serde(default)]
    pub shape_name: Option<String>,
    /// Bandwidth of a flexible load balancer.
    #[serde(default)]
    pub shape_details: Option<LbShape>,
    /// `ACTIVE`, `DELETED`, ...
    #[serde(default)]
    pub lifecycle_state: Option<String>,
    /// Region it lives in (set by `collect`).
    #[serde(skip)]
    pub region: String,
}

/// Bandwidth of a flexible load balancer, Mbit/s.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LbShape {
    /// Minimum.
    pub minimum_bandwidth_in_mbps: Option<f64>,
    /// Maximum.
    pub maximum_bandwidth_in_mbps: Option<f64>,
}

impl LoadBalancer {
    /// The console name, or the end of the OCID.
    pub fn name(&self) -> String {
        name_or_ocid(self.display_name.as_deref(), &self.id, "load balancer")
    }

    /// Not deleted.
    pub fn exists(&self) -> bool {
        !matches!(
            self.lifecycle_state.as_deref(),
            Some("DELETED" | "DELETING")
        )
    }

    /// The Always Free kind: flexible at 10 Mbit/s, or the old `10Mbps-Micro` shape.
    pub fn is_free_shape(&self) -> bool {
        match self.shape_name.as_deref() {
            Some("10Mbps-Micro") => true,
            Some("flexible") => self
                .shape_details
                .as_ref()
                .and_then(|d| d.maximum_bandwidth_in_mbps)
                .is_some_and(|max| max <= crate::limits::LB_FREE_MBPS),
            _ => false,
        }
    }
}
