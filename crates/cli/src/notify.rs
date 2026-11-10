//! `--notify`: tell the user when the audit's result changes — a new charge, a limit crossed,
//! or all free again. The last result is kept in the cache folder, so a daily cron job sends
//! one notification per change instead of one a day.

use std::path::PathBuf;
use std::time::Duration;

use oci_free_audit_core::{Report, Status};

/// Compare with the last run, notify if the problems changed, remember this run.
pub fn on_change(
    report: &Report,
    tenancy: &str,
    profile: &str,
    webhook: Option<&str>,
    desktop: bool,
) {
    let now = problems(report);
    let path = state_file(tenancy, profile);
    let before: Option<Vec<String>> = path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| text.lines().map(str::to_owned).collect());
    if let Some(text) = message(before.as_deref(), &now) {
        send(&text, report.result == Status::Fail, webhook, desktop);
    }
    if let Some(path) = path {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, now.join("\n"));
    }
}

/// Every ⚠ and ✗ line, e.g. `✗ COST · November 2026: 4.30 EUR`.
fn problems(report: &Report) -> Vec<String> {
    report
        .sections
        .iter()
        .flat_map(|s| s.findings.iter().map(move |f| (s, f)))
        .filter(|(_, f)| f.status != Status::Ok)
        .map(|(s, f)| {
            format!(
                "{} {} · {}: {}",
                f.status.symbol(),
                s.title,
                f.label,
                f.detail
            )
        })
        .collect()
}

/// What to say, if anything. The first run with no problems says nothing; with problems it
/// lists them.
fn message(before: Option<&[String]>, now: &[String]) -> Option<String> {
    let before = before.unwrap_or_default();
    if before == now {
        return None;
    }
    if now.is_empty() {
        return Some("oci-free-audit: all free again".to_owned());
    }
    let new: Vec<&String> = now.iter().filter(|p| !before.contains(p)).collect();
    let shown = if new.is_empty() {
        now.iter().collect()
    } else {
        new
    };
    let mut text = format!(
        "oci-free-audit: {} problem{}",
        now.len(),
        if now.len() == 1 { "" } else { "s" }
    );
    for p in shown.iter().take(5) {
        text.push('\n');
        text.push_str(p);
    }
    if shown.len() > 5 {
        text.push_str(&format!("\n… and {} more", shown.len() - 5));
    }
    Some(text)
}

/// One state file per tenancy and profile (another user may see less), in the cache folder.
fn state_file(tenancy: &str, profile: &str) -> Option<PathBuf> {
    let tail: String = tenancy
        .chars()
        .rev()
        .take(12)
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let profile: String = profile
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    dirs::cache_dir().map(|d| {
        d.join("oci-free-audit")
            .join(format!("last-{tail}-{profile}.txt"))
    })
}

/// Desktop and webhook. Failures are reported on stderr, never fatal.
fn send(text: &str, urgent: bool, webhook: Option<&str>, desktop: bool) {
    if desktop {
        let (title, body) = text.split_once('\n').unwrap_or((text, ""));
        let title = if urgent {
            format!("{title} — you may be charged")
        } else {
            title.to_owned()
        };
        let _ = notify_rust::Notification::new()
            .summary(&title)
            .body(body)
            .show();
    }
    if let Some(url) = webhook {
        let (content_type, body) = webhook_body(url, text);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .user_agent(concat!("oci-free-audit/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        if let Err(e) = agent
            .post(url)
            .header("content-type", content_type)
            .send(body.as_bytes())
        {
            eprintln!("webhook failed: {e}");
        }
    }
}

/// What each kind of webhook wants: ntfy takes plain text, Discord `content`, the rest `text`.
fn webhook_body(url: &str, text: &str) -> (&'static str, String) {
    if url.contains("ntfy") {
        ("text/plain", text.to_owned())
    } else if url.contains("discord.com") || url.contains("discordapp.com") {
        (
            "application/json",
            serde_json::json!({ "content": text }).to_string(),
        )
    } else {
        (
            "application/json",
            serde_json::json!({ "text": text }).to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn first_run_all_free_is_quiet() {
        assert_eq!(message(None, &[]), None);
    }

    #[test]
    fn same_problems_are_not_repeated() {
        let p = v(&["✗ COST · November 2026: 4.30 EUR"]);
        assert!(message(None, &p).unwrap().contains("4.30 EUR"));
        assert_eq!(message(Some(&p), &p), None);
    }

    #[test]
    fn only_new_problems_are_listed() {
        let before = v(&["⚠ STORAGE · Backups: 5 of 5"]);
        let now = v(&["⚠ STORAGE · Backups: 5 of 5", "✗ COMPUTE · big: E4"]);
        let text = message(Some(&before), &now).unwrap();
        assert!(text.starts_with("oci-free-audit: 2 problems"));
        assert!(text.contains("big") && !text.contains("Backups"));
    }

    #[test]
    fn all_free_again() {
        let before = v(&["✗ COST · November 2026: 4.30 EUR"]);
        assert_eq!(
            message(Some(&before), &[]).unwrap(),
            "oci-free-audit: all free again"
        );
    }

    #[test]
    fn one_state_file_per_tenancy_and_profile() {
        let a = state_file("ocid1.tenancy.oc1..aaaa1111", "DEFAULT").unwrap();
        assert_ne!(
            a,
            state_file("ocid1.tenancy.oc1..aaaa1111", "PROBE").unwrap()
        );
        assert_ne!(
            a,
            state_file("ocid1.tenancy.oc1..aaaa2222", "DEFAULT").unwrap()
        );
    }

    #[test]
    fn webhook_payloads() {
        assert_eq!(
            webhook_body("https://ntfy.sh/t", "hi"),
            ("text/plain", "hi".into())
        );
        assert_eq!(
            webhook_body("https://discord.com/api/webhooks/1/x", "hi").1,
            r#"{"content":"hi"}"#
        );
        assert_eq!(
            webhook_body("https://hooks.slack.com/services/x", "hi").1,
            r#"{"text":"hi"}"#
        );
    }
}
