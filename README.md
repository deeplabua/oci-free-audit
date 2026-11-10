# oci-free-audit

**Will Oracle charge me?** A read-only check of your Oracle Cloud (OCI) tenancy against the
Always Free limits — and against what you were actually billed.

One binary, no OCI CLI, no Python. It reads your account, compares every resource with the
Always Free allowance, looks at the real cost in Oracle's Usage API, and prints ✓ / ⚠ / ✗.
**It never changes anything in the account.**

Part of [DeepLab](https://deeplab.tools) — small, sharp tools that run locally. A sibling of
[oci-arm-catcher](https://github.com/deeplabua/oci-arm-catcher): catch a free ARM instance,
then make sure it stays free.

```
oci-free-audit · tenancy ****xyz · PAYG · home region eu-amsterdam-1 · since 2026-10-01 · limits as of 2026-11-10

COST  (data up to 2026-11-09)
  ✓ November 2026 to date      0.00 EUR
  ✓ October 2026               0.00 EUR
  ✓ Outbound data this month   1.3 GB of 10 TB

COMPUTE
  ✓ A1.Flex total   4 OCPU / 24 GB, over 2 OCPU / 12 GB, but free on this account: A1 billed 0.00 EUR since 2026-10-01
      arm-free-1: 4 OCPU / 24 GB
  ✓ E2.1.Micro      1 of 2

STORAGE
  ✓ Block volumes    94 GB of 200 (boot 94, block 0)
  ✓ Volume backups   0 of 5

SAFETY NET
  ✓ Budget alert   "zero-spend" with 1 alert rule(s)

Result: all free
```

## Why

- The usual advice for getting ARM capacity is "upgrade to Pay-As-You-Go". Then a card is on
  file, and a forgotten resource turns into a bill.
- Limits change quietly: on 15 June 2026 Oracle halved the Always Free A1 allowance from
  4 OCPU / 24 GB to 2 OCPU / 12 GB. Some PAYG accounts still pay nothing for 4/24, some may not.
- The console spreads the answer over Limits, Cost Analysis and the create forms. There is no
  single "is everything free?" page.

So the audit trusts money first: if the Usage API shows a non-zero cost, that is a ✗, whatever
the inventory says. Between 2/12 and 4/24 of A1 it lets your own bill decide.

## Install

Not released yet. From source:

```sh
cargo install --git https://github.com/deeplabua/oci-free-audit oci-free-audit
```

It uses the API key in `~/.oci/config` — the same one the OCI CLI and
[oci-arm-catcher](https://github.com/deeplabua/oci-arm-catcher) use.

## Use

```sh
oci-free-audit                    # the report
oci-free-audit --json             # for scripts and cron
oci-free-audit --since 2026-09-01 # audit a longer period (default: since the 1st of last month)
oci-free-audit --a1-limit 4/24    # treat 4 OCPU / 24 GB of A1 as free instead of asking the bill
oci-free-audit --show-ids         # don't mask the tenancy name and OCID
oci-free-audit --profile OTHER    # another profile in ~/.oci/config
```

Exit codes: `0` all free, `1` warnings, `2` a charge or a limit exceeded, `3` could not run
(arguments, config, key, network). Put it in cron and alert on non-zero.

The tenancy name and OCID are masked by default, so the report is safe to paste into an issue
or a forum post.

## What it checks

Limits as of 10 November 2026, from
[Always Free Resources](https://docs.oracle.com/iaas/Content/FreeTier/resourceref.htm):

| Area | Rule |
|---|---|
| Cost | any non-zero cost in the Usage API, per month, by service → ✗ |
| Outbound data | ≤ 10 TB per month (⚠ above 80 %) |
| Compute shapes | only `VM.Standard.A1.Flex` and `VM.Standard.E2.1.Micro` are free; anything else running → ✗ |
| A1 | ≤ 2 OCPU / 12 GB ✓; up to 4 / 24 ✓ only if the bill for A1 is zero; above → ✗ |
| E2.1.Micro | ≤ 2 instances |
| Regions | compute and block volumes outside the home region → ✗ (never free there) |
| Block storage | boot + block volumes ≤ 200 GB in the home region (⚠ from 90 %) |
| Volume backups | boot + block ≤ 5 (⚠ at 5: no room left) |
| Object Storage | ≤ 20 GB on an Always Free account; ≤ 10 GB each of Standard and Archive on PAYG |
| Autonomous Database | ≤ 2 with `isFreeTier`; any other running → ✗, stopped → ⚠ (storage billed) |
| MySQL HeatWave | ≤ 1 `MySQL.Free`; any other shape running → ✗ |
| Load balancer | ≤ 1 flexible at 10 Mbit/s; any other shape → ✗ |
| Network load balancer | ≤ 1 (⚠ above) |
| VCN | ≤ 2 on an Always Free account (⚠ above); no cap on PAYG |
| Vault | virtual vaults are free; a virtual private vault → ✗ |
| Budget | a budget with an alert rule exists — otherwise ⚠ and how to create one |

Every subscribed region and every compartment is checked. Anything the API refuses to show is a
⚠ "could not check", never a silent ✓.

## Permissions

Read-only. A user or group with this policy is enough (checked on a real tenancy, 10 November
2026):

```
Allow group <your-group> to inspect all-resources in tenancy
Allow group <your-group> to read usage-report in tenancy
```

With only `inspect`, OCI hides resource names; the report then shows the end of the OCID
instead. Add `Allow group <your-group> to read instances in tenancy` to see instance names, and
`Allow group <your-group> to read buckets in tenancy` for Object Storage sizes.

## Not yet

Vault key versions and secrets counts, Email Delivery and Notifications volumes, the 50,000
Object Storage requests a month, the trial period, notifications. See [`CHANGELOG.md`](CHANGELOG.md).

## License

[MIT](LICENSE)
