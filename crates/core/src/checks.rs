//! The checks: a [`Snapshot`] in, a [`Report`] out. No network, so every rule is unit-tested.

use std::collections::BTreeMap;

use crate::date::Date;
use crate::limits::{self, Allowance};
use crate::model::{Service, Snapshot, UsageItem};
use crate::report::{Finding, Report, Section, Status};

/// Cost below this is treated as zero (rounding in the Usage API).
const ZERO: f64 = 0.005;

/// What the user can change.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Treat this much A1 as free (`--a1-limit 4/24`) instead of judging by the bill.
    pub a1_limit: Option<Allowance>,
}

/// Run every check.
pub fn audit(s: &Snapshot, opts: &Options) -> Report {
    let sections = vec![
        Section {
            title: "COST".into(),
            findings: cost(s),
        },
        Section {
            title: "COMPUTE".into(),
            findings: compute(s, opts),
        },
        Section {
            title: "STORAGE".into(),
            findings: storage(s),
        },
        Section {
            title: "DATABASES".into(),
            findings: databases(s),
        },
        Section {
            title: "NETWORK".into(),
            findings: network(s),
        },
        Section {
            title: "SECURITY".into(),
            findings: security(s),
        },
        Section {
            title: "SAFETY NET".into(),
            findings: safety_net(s),
        },
    ];
    let result = sections
        .iter()
        .flat_map(|sec| &sec.findings)
        .map(|f| f.status)
        .max()
        .unwrap_or(Status::Ok);
    Report {
        tenancy: s.tenancy_name.clone(),
        tenancy_id: s.tenancy_id.clone(),
        home_region: s.home_region.clone(),
        payment_model: s.payment_model.clone(),
        limits_as_of: limits::CHECKED,
        since: s.since,
        today: s.today,
        cost_data_up_to: s
            .usage
            .as_ref()
            .ok()
            .and_then(|items| items.iter().filter_map(UsageItem::date).max()),
        sections,
        result,
    }
}

fn money(amount: f64, currency: &str) -> String {
    format!("{amount:.2} {currency}").trim_end().to_owned()
}

/// §5.1: what Oracle actually billed, per month, and outbound traffic.
fn cost(s: &Snapshot) -> Vec<Finding> {
    let items = match &s.usage {
        Ok(items) => items,
        Err(e) => return vec![Finding::unknown("Billed cost", e)],
    };
    let currency = items
        .iter()
        .find_map(|i| i.currency.clone())
        .unwrap_or_default();

    // Every month from `since` to today, even those without rows.
    let mut months: BTreeMap<String, (Date, f64, BTreeMap<String, f64>)> = BTreeMap::new();
    let mut m = s.since.first_of_month();
    while m <= s.today {
        months.insert(m.month_key(), (m, 0.0, BTreeMap::new()));
        m = Date::from_days(m.to_days() + 32).first_of_month();
    }
    for item in items {
        let Some(date) = item.date() else { continue };
        if let Some((_, total, by_service)) = months.get_mut(&date.month_key()) {
            *total += item.amount();
            *by_service
                .entry(item.service.clone().unwrap_or_else(|| "Other".into()))
                .or_default() += item.amount();
        }
    }

    let current = s.today.month_key();
    let mut findings: Vec<Finding> = months
        .into_values()
        .rev()
        .map(|(month, total, by_service)| {
            let label = if month.month_key() == current {
                format!("{} to date", month.month_name())
            } else {
                month.month_name()
            };
            if total > ZERO {
                let mut items: Vec<(String, f64)> =
                    by_service.into_iter().filter(|(_, a)| *a > ZERO).collect();
                items.sort_by(|a, b| b.1.total_cmp(&a.1));
                Finding::new(Status::Fail, label, money(total, &currency)).with_items(
                    items
                        .into_iter()
                        .map(|(svc, a)| format!("{svc}: {}", money(a, &currency)))
                        .collect(),
                )
            } else {
                Finding::new(Status::Ok, label, money(0.0, &currency))
            }
        })
        .collect();

    let egress: f64 = items
        .iter()
        .filter(|i| i.date().is_some_and(|d| d.month_key() == current))
        .filter(|i| {
            i.sku_name
                .as_deref()
                .is_some_and(|n| n.starts_with(limits::EGRESS_SKU_PREFIX))
        })
        .map(|i| i.computed_quantity.unwrap_or(0.0))
        .sum::<f64>()
        + 0.0;
    let share = egress / limits::EGRESS_GB_PER_MONTH;
    let status = if share > 1.0 {
        Status::Fail
    } else if share > 0.8 {
        Status::Warn
    } else {
        Status::Ok
    };
    findings.push(Finding::new(
        status,
        "Outbound data this month",
        format!("{egress:.1} GB of 10 TB"),
    ));
    findings
}

/// §5.2 compute: shapes, the A1 allowance, E2.1.Micro count.
fn compute(s: &Snapshot, opts: &Options) -> Vec<Finding> {
    let mut out = Vec::new();
    let all = match &s.instances {
        Ok(list) => list.iter().filter(|i| i.exists()).collect::<Vec<_>>(),
        Err(e) => {
            out.push(Finding::unknown("Instances", e));
            return out;
        }
    };
    let in_home = |region: &str| region.is_empty() || region == s.home_region;
    // Compute is free only in the home region.
    for i in all.iter().filter(|i| !in_home(&i.region)) {
        out.push(if i.running() {
            Finding::new(
                Status::Fail,
                i.name(),
                format!(
                    "{} in {} — compute is free only in the home region ({})",
                    i.shape, i.region, s.home_region
                ),
            )
        } else {
            Finding::new(
                Status::Warn,
                i.name(),
                format!(
                    "{} in {} ({}) — outside the home region; its boot volume is billed",
                    i.shape,
                    i.region,
                    i.lifecycle_state.to_lowercase()
                ),
            )
        });
    }
    let instances: Vec<_> = all.into_iter().filter(|i| in_home(&i.region)).collect();

    for i in instances
        .iter()
        .filter(|i| !limits::is_free_shape(&i.shape))
    {
        out.push(if i.running() {
            Finding::new(
                Status::Fail,
                i.name(),
                format!("{} is not Always Free", i.shape),
            )
        } else {
            Finding::new(
                Status::Warn,
                i.name(),
                format!(
                    "{} ({}) — no compute charge while stopped; its boot volume still counts",
                    i.shape,
                    i.lifecycle_state.to_lowercase()
                ),
            )
        });
    }

    let a1: Vec<_> = instances
        .iter()
        .filter(|i| i.shape == limits::A1_FLEX && i.running())
        .collect();
    let total = Allowance {
        ocpus: a1.iter().map(|i| i.ocpus()).sum::<f64>() + 0.0,
        memory_gb: a1.iter().map(|i| i.memory_gb()).sum::<f64>() + 0.0,
    };
    let names: Vec<String> = a1
        .iter()
        .map(|i| format!("{}: {} OCPU / {} GB", i.name(), i.ocpus(), i.memory_gb()))
        .collect();
    let label = "A1.Flex total";
    out.push(if a1.is_empty() {
        Finding::new(Status::Ok, label, "none running")
    } else if let Some(limit) = opts.a1_limit {
        let status = if limit.covers(total) {
            Status::Ok
        } else {
            Status::Fail
        };
        Finding::new(
            status,
            label,
            format!("{total}, limit {limit} (--a1-limit)"),
        )
        .with_items(names)
    } else if limits::A1_FREE.covers(total) {
        Finding::new(Status::Ok, label, format!("{total} of {}", limits::A1_FREE)).with_items(names)
    } else if limits::A1_LEGACY.covers(total) {
        a1_between_limits(s, total).with_items(names)
    } else {
        Finding::new(
            Status::Fail,
            label,
            format!(
                "{total}, limit {} — above any free allowance",
                limits::A1_FREE
            ),
        )
        .with_items(names)
    });

    let micro = instances
        .iter()
        .filter(|i| i.shape == limits::E2_MICRO)
        .count();
    out.push(Finding::new(
        if micro > limits::E2_MICRO_MAX {
            Status::Fail
        } else {
            Status::Ok
        },
        "E2.1.Micro",
        format!("{micro} of {}", limits::E2_MICRO_MAX),
    ));
    out
}

/// Between 2/12 and 4/24 A1 is free on some accounts: the bill decides (PRD, question 2).
fn a1_between_limits(s: &Snapshot, total: Allowance) -> Finding {
    let label = "A1.Flex total";
    let over = format!("{total}, over {}", limits::A1_FREE);
    let rows: Vec<&UsageItem> = match &s.usage {
        Ok(items) => items
            .iter()
            .filter(|i| {
                i.sku_name
                    .as_deref()
                    .is_some_and(|n| limits::A1_SKUS.contains(&n))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    if rows.is_empty() {
        return Finding::new(
            Status::Warn,
            label,
            format!("{over}; free on some pay-as-you-go accounts — check the cost above"),
        );
    }
    let billed: f64 = rows.iter().map(|i| i.amount()).sum();
    let currency = rows
        .iter()
        .find_map(|i| i.currency.clone())
        .unwrap_or_default();
    if billed > ZERO {
        Finding::new(
            Status::Fail,
            label,
            format!(
                "{over}; A1 billed {} since {}",
                money(billed, &currency),
                s.since
            ),
        )
    } else {
        Finding::new(
            Status::Ok,
            label,
            format!(
                "{over}, but free on this account: A1 billed {} since {}",
                money(0.0, &currency),
                s.since
            ),
        )
    }
}

/// §5.2 storage: volumes against 200 GB, backups against 5.
fn storage(s: &Snapshot) -> Vec<Finding> {
    let mut out = Vec::new();
    match (&s.boot_volumes, &s.block_volumes) {
        (Ok(boot), Ok(block)) => {
            let home = |v: &crate::model::Volume| v.region.is_empty() || v.region == s.home_region;
            let outside: Vec<String> = boot
                .iter()
                .chain(block)
                .filter(|v| v.exists() && !home(v))
                .map(|v| format!("{} GB in {}", v.size_in_gbs.unwrap_or(0.0), v.region))
                .collect();
            if !outside.is_empty() {
                out.push(
                    Finding::new(
                        Status::Fail,
                        "Volumes outside the home region",
                        format!(
                            "{} — block storage is free only in {}",
                            outside.len(),
                            s.home_region
                        ),
                    )
                    .with_items(outside),
                );
            }
            let gb = |v: &Vec<crate::model::Volume>| -> f64 {
                // + 0.0: an empty f64 sum is -0.0.
                v.iter()
                    .filter(|v| v.exists() && home(v))
                    .filter_map(|v| v.size_in_gbs)
                    .sum::<f64>()
                    + 0.0
            };
            let (b, k) = (gb(boot), gb(block));
            let total = b + k;
            let status = if total > limits::BLOCK_STORAGE_GB {
                Status::Fail
            } else if total >= limits::BLOCK_STORAGE_GB * 0.9 {
                Status::Warn
            } else {
                Status::Ok
            };
            out.push(Finding::new(
                status,
                "Block volumes",
                format!(
                    "{total} GB of {} (boot {b}, block {k})",
                    limits::BLOCK_STORAGE_GB
                ),
            ));
        }
        (Err(e), _) | (_, Err(e)) => out.push(Finding::unknown("Block volumes", e)),
    }
    match &s.backups {
        Ok(list) => {
            let n = list.iter().filter(|b| b.exists()).count();
            let (status, note) = match n.cmp(&limits::VOLUME_BACKUPS) {
                std::cmp::Ordering::Greater => (Status::Fail, ""),
                std::cmp::Ordering::Equal => (Status::Warn, " — no room left"),
                std::cmp::Ordering::Less => (Status::Ok, ""),
            };
            out.push(Finding::new(
                status,
                "Volume backups",
                format!("{n} of {}{note}", limits::VOLUME_BACKUPS),
            ));
        }
        Err(e) => out.push(Finding::unknown("Volume backups", e)),
    }
    out.push(object_storage(s));
    out
}

/// Object Storage: 20 GB on an Always Free account; 10 GB each of Standard and Archive on PAYG.
fn object_storage(s: &Snapshot) -> Finding {
    let label = "Object Storage";
    let buckets = match &s.buckets {
        Ok(b) => b,
        Err(e) => return Finding::unknown(label, e),
    };
    if buckets.is_empty() {
        return Finding::new(Status::Ok, label, "no buckets");
    }
    let unknown: Vec<String> = buckets
        .iter()
        .filter(|b| b.approximate_size.is_none())
        .map(|b| b.name.clone())
        .collect();
    if !unknown.is_empty() {
        return Finding::new(
            Status::Warn,
            label,
            format!(
                "{} bucket(s), size unknown — reading sizes needs `read buckets`",
                buckets.len()
            ),
        )
        .with_items(unknown);
    }
    let gb = |tier: Option<&str>| -> f64 {
        buckets
            .iter()
            .filter(|b| tier.is_none_or(|t| b.storage_tier.as_deref().unwrap_or("Standard") == t))
            .filter_map(|b| b.approximate_size)
            .sum::<f64>()
            / 1e9
            + 0.0
    };
    let grade = |used: f64, limit: f64| {
        if used > limit {
            Status::Fail
        } else if used >= limit * 0.9 {
            Status::Warn
        } else {
            Status::Ok
        }
    };
    if s.payment_model.as_deref() == Some("PAYG") {
        let (std, arch) = (gb(Some("Standard")), gb(Some("Archive")));
        let lim = limits::OBJECT_STORAGE_PAYG_GB_PER_TIER;
        Finding::new(
            grade(std, lim).max(grade(arch, lim)),
            label,
            format!(
                "Standard {std:.1} GB of {lim}, Archive {arch:.1} GB of {lim} ({} buckets)",
                buckets.len()
            ),
        )
    } else {
        let total = gb(None);
        let lim = limits::OBJECT_STORAGE_GB;
        Finding::new(
            grade(total, lim),
            label,
            format!("{total:.1} GB of {lim} ({} buckets)", buckets.len()),
        )
    }
}

/// Autonomous Database and MySQL HeatWave.
fn databases(s: &Snapshot) -> Vec<Finding> {
    let mut out = Vec::new();
    match &s.autonomous_dbs {
        Err(e) => out.push(Finding::unknown("Autonomous Database", e)),
        Ok(list) => {
            let live: Vec<&Service> = list.iter().filter(|d| d.exists()).collect();
            for d in live.iter().filter(|d| d.is_free_tier != Some(true)) {
                out.push(paid_service(
                    &d.name("database"),
                    &d.region,
                    d.lifecycle_state.as_deref(),
                    "STOPPED",
                    "not Always Free",
                ));
            }
            let free = live.iter().filter(|d| d.is_free_tier == Some(true)).count();
            out.push(count(
                "Autonomous Database (free tier)",
                free,
                limits::AUTONOMOUS_DB_MAX,
            ));
        }
    }
    match &s.mysql {
        Err(e) => out.push(Finding::unknown("MySQL HeatWave", e)),
        Ok(list) => {
            let live: Vec<&Service> = list.iter().filter(|d| d.exists()).collect();
            let is_free = |d: &Service| d.shape_name.as_deref() == Some(limits::MYSQL_FREE_SHAPE);
            for d in live.iter().filter(|d| !is_free(d)) {
                let shape = d.shape_name.as_deref().unwrap_or("unknown shape");
                out.push(paid_service(
                    &d.name("DB system"),
                    &d.region,
                    d.lifecycle_state.as_deref(),
                    "INACTIVE",
                    &format!("{shape} is not Always Free"),
                ));
            }
            out.push(count(
                "MySQL HeatWave (free)",
                live.iter().filter(|d| is_free(d)).count(),
                limits::MYSQL_MAX,
            ));
        }
    }
    out
}

/// Load balancers, network load balancers, VCNs.
fn network(s: &Snapshot) -> Vec<Finding> {
    let mut out = Vec::new();
    match &s.load_balancers {
        Err(e) => out.push(Finding::unknown("Load balancers", e)),
        Ok(list) => {
            let live: Vec<_> = list.iter().filter(|l| l.exists()).collect();
            for l in live.iter().filter(|l| !l.is_free_shape()) {
                let shape = l.shape_name.as_deref().unwrap_or("unknown shape");
                out.push(Finding::new(
                    Status::Fail,
                    l.name(),
                    format!(
                        "{shape} load balancer in {} — only a flexible one at {} Mbit/s is free",
                        l.region,
                        limits::LB_FREE_MBPS
                    ),
                ));
            }
            out.push(count(
                "Load balancer (10 Mbit/s)",
                live.iter().filter(|l| l.is_free_shape()).count(),
                limits::LB_MAX,
            ));
        }
    }
    match &s.network_load_balancers {
        Err(e) => out.push(Finding::unknown("Network load balancers", e)),
        Ok(list) => {
            let n = list.iter().filter(|l| l.exists()).count();
            let status = if n > limits::NLB_MAX {
                Status::Warn
            } else {
                Status::Ok
            };
            out.push(Finding::new(
                status,
                "Network load balancers",
                format!("{n} of {}", limits::NLB_MAX),
            ));
        }
    }
    match &s.vcns {
        Err(e) => out.push(Finding::unknown("VCNs", e)),
        Ok(list) => {
            let n = list.iter().filter(|v| v.exists()).count();
            let payg = s.payment_model.as_deref() == Some("PAYG");
            let status = if !payg && n > limits::VCN_MAX {
                Status::Warn
            } else {
                Status::Ok
            };
            let detail = if payg {
                format!("{n} (no Always Free cap on PAYG)")
            } else {
                format!("{n} of {}", limits::VCN_MAX)
            };
            out.push(Finding::new(status, "VCNs", detail));
        }
    }
    out
}

/// Vault: virtual vaults are free, a virtual private vault is billed.
fn security(s: &Snapshot) -> Vec<Finding> {
    vec![match &s.vaults {
        Err(e) => Finding::unknown("Vaults", e),
        Ok(list) => {
            let live: Vec<_> = list.iter().filter(|v| v.exists()).collect();
            let private: Vec<String> = live
                .iter()
                .filter(|v| v.vault_type.as_deref() == Some("VIRTUAL_PRIVATE"))
                .map(|v| format!("{} in {}", v.name("vault"), v.region))
                .collect();
            if private.is_empty() {
                Finding::new(
                    Status::Ok,
                    "Vaults",
                    format!("{} virtual (free)", live.len()),
                )
            } else {
                Finding::new(
                    Status::Fail,
                    "Vaults",
                    format!("{} virtual private vault(s) — billed", private.len()),
                )
                .with_items(private)
            }
        }
    }]
}

/// A resource that is not Always Free: ✗ while running, ⚠ while stopped (storage still billed).
fn paid_service(
    name: &str,
    region: &str,
    state: Option<&str>,
    stopped_state: &str,
    why: &str,
) -> Finding {
    if state == Some(stopped_state) {
        Finding::new(
            Status::Warn,
            name,
            format!("{why}, in {region}; stopped — storage is still billed"),
        )
    } else {
        Finding::new(Status::Fail, name, format!("{why}, in {region}"))
    }
}

/// `n of max`: ✗ above the free count.
fn count(label: &str, n: usize, max: usize) -> Finding {
    Finding::new(
        if n > max { Status::Fail } else { Status::Ok },
        label,
        format!("{n} of {max}"),
    )
}

/// §5.3: is anyone told when money starts flowing?
fn safety_net(s: &Snapshot) -> Vec<Finding> {
    const HINT: &str = "create one with an email alert at https://cloud.oracle.com/usage/budgets (e.g. 1.00, alert at 100% actual)";
    let label = "Budget alert";
    vec![match &s.budgets {
        Err(e) => Finding::unknown(label, e),
        Ok(budgets) if budgets.is_empty() => {
            Finding::new(Status::Warn, label, "no budget").with_items(vec![HINT.into()])
        }
        Ok(budgets) => match budgets.iter().find(|b| b.alert_rule_count.unwrap_or(0) > 0) {
            Some(b) => Finding::new(
                Status::Ok,
                label,
                format!(
                    "\"{}\" with {} alert rule(s)",
                    b.display_name,
                    b.alert_rule_count.unwrap_or(0)
                ),
            ),
            None => Finding::new(
                Status::Warn,
                label,
                format!("{} budget(s), none with an alert rule", budgets.len()),
            )
            .with_items(vec![HINT.into()]),
        },
    }]
}
