//! The report as text, and masking for sharing it.

use std::fmt::Write as _;

use oci_free_audit_core::{Report, Status};

/// Hide the tenancy name and OCID, so the report can go into an issue or a forum post.
pub fn masked(mut report: Report) -> Report {
    report.tenancy = mask_tail(&report.tenancy, 3);
    report.tenancy_id = match report.tenancy_id.rsplit_once('.') {
        Some((head, tail)) => format!("{head}.{}", mask_tail(tail, 4)),
        None => mask_tail(&report.tenancy_id, 4),
    };
    report
}

/// `****xyz`: only the last `keep` characters.
fn mask_tail(text: &str, keep: usize) -> String {
    let n = text.chars().count();
    if n == 0 {
        return String::new();
    }
    let tail: String = text.chars().skip(n.saturating_sub(keep)).collect();
    format!("****{tail}")
}

/// The human-readable report.
pub fn text(r: &Report) -> String {
    let mut out = String::new();
    let mut head = format!("oci-free-audit · tenancy {}", r.tenancy);
    if let Some(model) = &r.payment_model {
        head += &format!(" · {model}");
    }
    let _ = writeln!(
        out,
        "{head} · home region {} · since {} · limits as of {}",
        r.home_region, r.since, r.limits_as_of
    );
    for section in &r.sections {
        let mut title = section.title.clone();
        if let (true, Some(up_to)) = (section.title == "COST", r.cost_data_up_to) {
            title += &format!("  (data up to {up_to})");
        }
        let _ = writeln!(out, "\n{title}");
        let width = section
            .findings
            .iter()
            .map(|f| f.label.chars().count())
            .max()
            .unwrap_or(0);
        for f in &section.findings {
            let pad = width - f.label.chars().count();
            let _ = writeln!(
                out,
                "  {} {}{}   {}",
                f.status.symbol(),
                f.label,
                " ".repeat(pad),
                f.detail
            );
            for item in &f.items {
                let _ = writeln!(out, "      {item}");
            }
        }
    }
    let (fail, warn) = (r.count(Status::Fail), r.count(Status::Warn));
    let summary = match (fail, warn) {
        (0, 0) => "all free".to_owned(),
        _ => format!(
            "{} problem{}, {} warning{}",
            fail,
            plural(fail),
            warn,
            plural(warn)
        ),
    };
    let _ = writeln!(out, "\nResult: {summary}");
    out
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_names_and_ocids() {
        assert_eq!(mask_tail("alexpua", 3), "****pua");
        assert_eq!(mask_tail("", 3), "");
        assert_eq!(mask_tail("ab", 3), "****ab");
    }
}
