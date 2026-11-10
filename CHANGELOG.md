# Changelog

## Unreleased — 0.1.0

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
