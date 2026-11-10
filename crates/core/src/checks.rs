//! The checks: a [`Snapshot`] in, a [`Report`] out. No network, so every rule is unit-tested.

use std::collections::BTreeMap;

use crate::date::Date;
use crate::limits::{self, Allowance};
use crate::model::{Snapshot, UsageItem};
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
    if s.regions.len() > 1 {
        let others: Vec<String> = s
            .regions
            .iter()
            .filter(|r| **r != s.home_region)
            .cloned()
            .collect();
        out.push(
            Finding::new(
                Status::Warn,
                "Other regions",
                format!(
                    "{} subscribed, not checked yet — compute there is never free",
                    others.len()
                ),
            )
            .with_items(others),
        );
    }
    let instances = match &s.instances {
        Ok(list) => list.iter().filter(|i| i.exists()).collect::<Vec<_>>(),
        Err(e) => {
            out.push(Finding::unknown("Instances", e));
            return out;
        }
    };

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
            let gb = |v: &Vec<crate::model::Volume>| -> f64 {
                // + 0.0: an empty f64 sum is -0.0.
                v.iter()
                    .filter(|v| v.exists())
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
    out
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
