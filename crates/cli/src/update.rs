//! A once-a-day check for a newer release, reported on stderr.
//!
//! Runs on a background thread so it never slows a command down: the result is waited for
//! briefly, and silently dropped if GitHub is slow or unreachable. The answer is cached for a
//! day, so most runs make no request at all. Off in CI, without a terminal on stderr, or with
//! `OCI_FREE_AUDIT_NO_UPDATE_CHECK` set. Same as oci-arm-catcher's.

use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const NAME: &str = "oci-free-audit";
const LATEST_URL: &str = "https://api.github.com/repos/deeplabua/oci-free-audit/releases/latest";
const RELEASES_URL: &str = "https://github.com/deeplabua/oci-free-audit/releases/latest";
const OPT_OUT_VAR: &str = "OCI_FREE_AUDIT_NO_UPDATE_CHECK";
const CURRENT: &str = env!("CARGO_PKG_VERSION");
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// A check in flight, reported at most once.
pub struct UpdateCheck {
    latest: Option<Receiver<Option<String>>>,
}

impl UpdateCheck {
    /// Start looking up the latest version, unless checks are off.
    pub fn start() -> Self {
        if !enabled() {
            return Self { latest: None };
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(latest_version());
        });
        Self { latest: Some(rx) }
    }

    /// Print a notice if a newer version is out, waiting at most `wait` for the answer.
    pub fn report(&mut self, wait: Duration) {
        let Some(rx) = self.latest.take() else {
            return;
        };
        let Ok(Some(latest)) = rx.recv_timeout(wait) else {
            return;
        };
        if is_newer(&latest, CURRENT) {
            eprintln!(
                "{}",
                notice(&latest, &upgrade_command(&current_exe()), color())
            );
        }
    }
}

/// The notice: what is new, the command to run (highlighted), where the release notes are.
fn notice(latest: &str, command: &str, color: bool) -> String {
    let paint = |code: &str, text: &str| {
        if color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    };
    format!(
        "\n{}\n  Update: {}\n  {}\n  {}\n",
        paint(
            "1;33",
            &format!("A new version of {NAME} is available: {CURRENT} → {latest}")
        ),
        paint("1;32", command),
        paint("2", &format!("Release notes: {RELEASES_URL}")),
        paint("2", &format!("Turn this check off: {OPT_OUT_VAR}=1")),
    )
}

fn enabled() -> bool {
    let set = |var: &str| std::env::var_os(var).is_some_and(|v| !v.is_empty());
    std::io::stderr().is_terminal() && !set(OPT_OUT_VAR) && !set("CI")
}

/// ANSI colour unless `NO_COLOR` is set (https://no-color.org); stderr is a terminal already.
fn color() -> bool {
    std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}

/// The latest released version: from the cache if it is fresh, else from GitHub.
fn latest_version() -> Option<String> {
    let cache = cache_file();
    let now = unix_now();
    if let Some((checked_at, latest)) = cache
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| parse_cache(&text))
        && is_fresh(checked_at, now)
    {
        return Some(latest);
    }
    let latest = fetch_latest()?;
    if let Some(path) = cache {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, format!("{now} {latest}\n"));
    }
    Some(latest)
}

fn fetch_latest() -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .user_agent(concat!("oci-free-audit/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let body = agent
        .get(LATEST_URL)
        .header("accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let release: serde_json::Value = serde_json::from_str(&body).ok()?;
    Some(
        release["tag_name"]
            .as_str()?
            .trim_start_matches('v')
            .to_owned(),
    )
}

fn cache_file() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join(NAME).join("latest-version"))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `"<unix seconds> <version>"`.
fn parse_cache(text: &str) -> Option<(u64, String)> {
    let (at, version) = text.trim().split_once(' ')?;
    Some((at.parse().ok()?, version.to_owned()))
}

fn is_fresh(checked_at: u64, now: u64) -> bool {
    now.saturating_sub(checked_at) < CHECK_EVERY.as_secs()
}

/// Compare dotted numeric versions; anything unparsable is never "newer".
fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(v: &str) -> Option<Vec<u64>> {
        v.split('.').map(|part| part.parse().ok()).collect()
    }
    match (parse(latest), parse(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

fn current_exe() -> PathBuf {
    std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_default()
}

/// How to upgrade, guessed from where the binary lives.
fn upgrade_command(exe: &Path) -> String {
    let path = exe.to_string_lossy();
    if path.contains("/Cellar/") || path.contains("/homebrew/") || path.contains("/linuxbrew/") {
        format!("brew upgrade deeplabua/tap/{NAME}")
    } else if path.contains("/.cargo/bin/") {
        format!("cargo install {NAME}")
    } else if cfg!(windows) {
        format!(
            "powershell -c \"irm https://github.com/deeplabua/{NAME}/releases/latest/download/{NAME}-installer.ps1 | iex\""
        )
    } else {
        format!(
            "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/deeplabua/{NAME}/releases/latest/download/{NAME}-installer.sh | sh"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(!is_newer("0.1.1", "0.1.2"));
        assert!(!is_newer("garbage", "0.1.2"));
        assert!(!is_newer("0.2.0-beta", "0.1.2"));
    }

    #[test]
    fn cache_round_trip_and_freshness() {
        assert_eq!(
            parse_cache("1700000000 0.1.3\n"),
            Some((1_700_000_000, "0.1.3".into()))
        );
        assert_eq!(parse_cache(""), None);
        assert_eq!(parse_cache("soon 0.1.3"), None);
        assert!(is_fresh(1000, 1000 + 3600));
        assert!(!is_fresh(1000, 1000 + 25 * 3600));
    }

    #[test]
    fn suggests_the_way_it_was_installed() {
        let brew = upgrade_command(Path::new(&format!(
            "/opt/homebrew/Cellar/{NAME}/0.1.0/bin/{NAME}"
        )));
        assert_eq!(brew, format!("brew upgrade deeplabua/tap/{NAME}"));
        let cargo = upgrade_command(Path::new(&format!("/home/me/.cargo/bin/{NAME}")));
        assert_eq!(cargo, format!("cargo install {NAME}"));
        let other = upgrade_command(Path::new(&format!("/usr/local/bin/{NAME}")));
        assert!(other.contains("installer"), "{other}");
    }

    #[test]
    fn the_command_is_highlighted() {
        let plain = notice("9.9.9", "brew upgrade x", false);
        assert!(plain.contains("Update: brew upgrade x"), "{plain}");
        assert!(!plain.contains('\x1b'));
        let coloured = notice("9.9.9", "brew upgrade x", true);
        assert!(
            coloured.contains("\x1b[1;32mbrew upgrade x\x1b[0m"),
            "{coloured}"
        );
    }
}

#[cfg(test)]
mod live {
    /// Hits GitHub: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn fetches_the_latest_release() {
        let latest = super::fetch_latest().expect("GitHub answered");
        assert!(latest.split('.').count() == 3, "{latest}");
    }
}
