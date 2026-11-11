# Changelog

## 0.4.1 — 2026-11-11

- When Oracle throttles requests (429), the audit waits as Oracle asks and retries up to
  3 times instead of failing. Built on oci-request-signer 0.2.

## 0.4.0 — 2026-11-10

- Vault: HSM-protected key versions (≤ 20) and secrets (≤ 150) are counted across every vault,
  region and compartment; software-protected keys are free and not counted.
- A MONTHLY section: this month's Object Storage requests (≤ 50,000), Email Delivery (≤ 3,000
  emails) and Notifications (≤ 1,000,000 HTTPS, ≤ 1,000 email deliveries) from the Usage API,
  ⚠ from 80 %.

## 0.3.0 — 2026-11-10

- Free trial: while the 30-day trial runs, a TRIAL section shows the day it ends and every
  resource that is not Always Free — stopped then, or billed if the account is already
  pay-as-you-go. Cost during the trial is ⚠ (it goes to the trial credits), not ✗.
- `--notify`: a desktop notification when the result changes — a new charge, a limit crossed,
  or all free again; the last result is kept in the cache folder. `--webhook` for ntfy, Slack,
  Discord or Telegram, `--no-desktop` for servers.
- Colour: ✓ green, ⚠ yellow, ✗ red — only in a terminal; `--color always|never`, `NO_COLOR`.

## 0.2.3 — 2026-11-10

- README: links to the other DeepLab OCI tools and the crate they share.
- README: install with Homebrew, the installer scripts or `cargo install`; how updating works.
- Output into a closed pipe (`… | head -1`) ends quietly with exit code 141 instead of a panic.

## 0.2.2 — 2026-11-10

- Update notice: at most once a day the binary checks GitHub for a newer release and
  prints how to upgrade — the command (brew, cargo or the installer) highlighted — on stderr.
  It never delays a run by more than 1.5 s; off in CI, without a terminal, or with
  `OCI_FREE_AUDIT_NO_UPDATE_CHECK=1`.

## 0.2.1 — 2026-11-10

- README: a GIF of a real run against a local mock (`assets/demo/`) instead of sample text.
- README: icon and badges (CI, crates.io, license, stars).
- README: a Support section — star the repo, or buy me a coffee.
- README: one command per code block, so each copies on its own.

## 0.2.0 — 2026-11-10

- Every subscribed region, not just the home one. Compute and block volumes outside the home
  region are ✗ — they are never free there.
- Object Storage: 20 GB on an Always Free account, 10 GB each of Standard and Archive on PAYG
  (sizes need `read buckets`; without it ⚠ says so).
- Autonomous Database (≤ 2 free tier; others ✗, stopped ⚠), MySQL HeatWave (≤ 1 `MySQL.Free`),
  load balancers (≤ 1 flexible at 10 Mbit/s), network load balancers, VCNs, Vault (a virtual
  private vault is billed).
- New report sections: DATABASES, NETWORK, SECURITY.

## 0.1.0 — 2026-11-10

First version. Read-only: only GET requests and the Usage API cost query.

- Cost: what the Usage API billed per month since `--since` (default: the 1st of last month),
  broken down by service; outbound data this month against 10 TB.
- Compute in the home region, all compartments: paid shapes, the A1 allowance (2/12; up to 4/24
  only if A1 was billed nothing — or `--a1-limit`), E2.1.Micro count.
- Storage: boot + block volumes against 200 GB, volume backups against 5.
- Safety net: a budget with an alert rule.
- Other subscribed regions are reported as not checked yet.
- `--json`, exit codes 0 / 1 / 2 / 3, tenancy name and OCID masked unless `--show-ids`.
- Always Free limits live in one module, `limits`, as of 2026-11-10.
