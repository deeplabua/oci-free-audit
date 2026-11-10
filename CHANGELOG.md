# Changelog

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
