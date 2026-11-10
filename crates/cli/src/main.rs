//! `oci-free-audit` — will Oracle charge me? A read-only check of an OCI tenancy against the
//! Always Free limits and the actual bill.
//!
//! Exit codes: 0 all free, 1 warnings, 2 charges or a limit exceeded, 3 could not run.

/// `println!` / `print!` that exit quietly when stdout is a closed pipe (`… | head -1`),
/// the way `cat` and `grep` do, instead of panicking. Declared before the modules so it
/// shadows the std macros everywhere in the binary.
macro_rules! println {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if let Err(e) = writeln!(std::io::stdout(), $($arg)*) {
            crate::stdout_failed(e);
        }
    }};
}
macro_rules! print {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if let Err(e) = write!(std::io::stdout(), $($arg)*) {
            crate::stdout_failed(e);
        }
    }};
}

/// A failed write to stdout: a closed pipe ends the program quietly with 141 (128 + SIGPIPE,
/// what a shell reports for a tool killed by SIGPIPE); anything else is a real error.
fn stdout_failed(e: std::io::Error) -> ! {
    if e.kind() == std::io::ErrorKind::BrokenPipe {
        std::process::exit(141);
    }
    panic!("failed printing to stdout: {e}");
}

mod output;
mod update;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result, anyhow};
use clap::Parser;
use oci_free_audit_core::limits::Allowance;
use oci_free_audit_core::{Date, Options, api, audit};
use oci_request_signer::{Client, Profile};

/// Exit code when the audit could not run: bad arguments, config, key or network.
const CANNOT_RUN: u8 = 3;

/// Will Oracle charge me? A read-only check of your OCI tenancy against the Always Free
/// limits — and against what you were actually billed.
///
/// Never changes anything in the account. Exit codes: 0 all free, 1 warnings, 2 charges or a
/// limit exceeded, 3 could not run.
#[derive(Debug, Parser)]
#[command(name = "oci-free-audit", version)]
struct Args {
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,

    /// Start of the audited period, YYYY-MM-DD [default: first day of last month].
    #[arg(long, env = "OCI_FREE_AUDIT_SINCE", value_name = "DATE")]
    since: Option<String>,

    /// Treat this much A1 as free, OCPUs/GB (e.g. 4/24), instead of judging by the bill.
    #[arg(long, env = "OCI_FREE_AUDIT_A1_LIMIT", value_name = "OCPU/GB")]
    a1_limit: Option<String>,

    /// Show the tenancy name and OCID instead of masking them.
    #[arg(long)]
    show_ids: bool,

    /// OCI config file [default: ~/.oci/config].
    #[arg(long, env = "OCI_CONFIG_FILE", value_name = "FILE")]
    oci_config: Option<PathBuf>,

    /// Profile in the OCI config file.
    #[arg(long, env = "OCI_CLI_PROFILE", default_value = "DEFAULT")]
    profile: String,
}

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            let _ = e.print();
            // --help and --version are not errors; everything else is "could not run",
            // not clap's default 2, which here means "you are being charged".
            return if e.use_stderr() {
                ExitCode::from(CANNOT_RUN)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    let mut update = update::UpdateCheck::start();
    let code = match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(CANNOT_RUN)
        }
    };
    update.report(std::time::Duration::from_millis(1500));
    code
}

fn run(args: &Args) -> Result<u8> {
    let today = Date::today();
    let since = match &args.since {
        Some(text) => Date::parse(text)
            .ok_or_else(|| anyhow!("--since: expected YYYY-MM-DD, got {text:?}"))?,
        None => today.previous_month(),
    };
    let a1_limit = match &args.a1_limit {
        Some(text) => Some(
            Allowance::parse(text)
                .ok_or_else(|| anyhow!("--a1-limit: expected OCPU/GB like 4/24, got {text:?}"))?,
        ),
        None => None,
    };

    let path = match &args.oci_config {
        Some(p) => p.clone(),
        None => {
            Profile::default_path().context("cannot find your home directory; pass --oci-config")?
        }
    };
    let profile = Profile::load(&path, &args.profile)?;
    let client = Client::builder(&profile)
        .user_agent(concat!("oci-free-audit/", env!("CARGO_PKG_VERSION")))
        // A local mock of the OCI API for tests and demos.
        .endpoint(std::env::var("OCI_FREE_AUDIT_ENDPOINT").ok())
        .build()?;

    let snapshot = api::collect(&client, since, today)?;
    let report = audit(&snapshot, &Options { a1_limit });
    let report = if args.show_ids {
        report
    } else {
        output::masked(report)
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", output::text(&report));
    }
    Ok(report.exit_code())
}
