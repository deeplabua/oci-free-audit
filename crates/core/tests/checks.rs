//! Every rule of the audit on hand-built snapshots.

use oci_free_audit_core::limits::Allowance;
use oci_free_audit_core::model::{
    Backup, Budget, Instance, ShapeConfig, Snapshot, UsageItem, Volume,
};
use oci_free_audit_core::{Date, Finding, Options, Report, Status, audit};

fn d(text: &str) -> Date {
    Date::parse(text).unwrap()
}

fn inst(name: &str, shape: &str, state: &str, ocpus: f64, mem: f64) -> Instance {
    Instance {
        id: format!("ocid1.instance.oc1..{name}"),
        display_name: Some(name.into()),
        shape: shape.into(),
        lifecycle_state: state.into(),
        shape_config: Some(ShapeConfig {
            ocpus: Some(ocpus),
            memory_in_gbs: Some(mem),
        }),
    }
}

fn vol(gb: f64) -> Volume {
    Volume {
        id: "ocid1.volume.oc1..v".into(),
        display_name: None,
        size_in_gbs: Some(gb),
        lifecycle_state: "AVAILABLE".into(),
    }
}

fn backup() -> Backup {
    Backup {
        id: "ocid1.backup.oc1..b".into(),
        display_name: None,
        lifecycle_state: "AVAILABLE".into(),
    }
}

fn usage(day: &str, service: &str, sku: &str, amount: f64, qty: f64) -> UsageItem {
    UsageItem {
        time_usage_started: format!("{day}T00:00:00.000Z"),
        service: Some(service.into()),
        sku_name: Some(sku.into()),
        computed_amount: Some(amount),
        computed_quantity: Some(qty),
        currency: Some("EUR".into()),
        unit: None,
    }
}

/// A free account: one E2.1.Micro, a 2/12 A1, 100 GB of volumes, a budget with an alert.
fn base() -> Snapshot {
    Snapshot {
        tenancy_name: "acme".into(),
        tenancy_id: "ocid1.tenancy.oc1..t".into(),
        home_region: "eu-amsterdam-1".into(),
        regions: vec!["eu-amsterdam-1".into()],
        payment_model: Some("PAYG".into()),
        instances: Ok(vec![
            inst("micro", "VM.Standard.E2.1.Micro", "RUNNING", 1.0, 1.0),
            inst("arm", "VM.Standard.A1.Flex", "RUNNING", 2.0, 12.0),
        ]),
        boot_volumes: Ok(vec![vol(50.0), vol(50.0)]),
        block_volumes: Ok(vec![]),
        backups: Ok(vec![backup()]),
        usage: Ok(vec![
            usage("2026-10-05", "Compute", "Standard - A1", 0.0, 48.0),
            usage("2026-11-09", "Compute", "Standard - A1", 0.0, 48.0),
        ]),
        budgets: Ok(vec![Budget {
            display_name: "guard".into(),
            amount: Some(1.0),
            alert_rule_count: Some(1),
        }]),
        today: d("2026-11-10"),
        since: d("2026-10-01"),
    }
}

fn run(s: &Snapshot) -> Report {
    audit(s, &Options::default())
}

fn find<'a>(r: &'a Report, label: &str) -> &'a Finding {
    r.sections
        .iter()
        .flat_map(|s| &s.findings)
        .find(|f| f.label == label)
        .unwrap_or_else(|| panic!("no finding {label:?} in {r:#?}"))
}

#[test]
fn a_free_account_is_all_green() {
    let r = run(&base());
    assert_eq!(r.result, Status::Ok, "{r:#?}");
    assert_eq!(r.exit_code(), 0);
    assert_eq!(find(&r, "November 2026 to date").detail, "0.00 EUR");
    assert_eq!(find(&r, "October 2026").status, Status::Ok);
    assert_eq!(
        find(&r, "A1.Flex total").detail,
        "2 OCPU / 12 GB of 2 OCPU / 12 GB"
    );
    assert_eq!(
        find(&r, "Block volumes").detail,
        "100 GB of 200 (boot 100, block 0)"
    );
    assert_eq!(r.cost_data_up_to, Some(d("2026-11-09")));
}

#[test]
fn any_billed_cost_fails_with_a_breakdown() {
    let mut s = base();
    s.usage = Ok(vec![
        usage("2026-10-03", "Compute", "Standard - E4 - OCPU", 1.20, 10.0),
        usage(
            "2026-10-04",
            "Block Storage",
            "Block Volume Storage",
            0.30,
            5.0,
        ),
        usage("2026-10-05", "Compute", "Standard - E4 - OCPU", 0.80, 10.0),
    ]);
    let r = run(&s);
    let oct = find(&r, "October 2026");
    assert_eq!(oct.status, Status::Fail);
    assert_eq!(oct.detail, "2.30 EUR");
    assert_eq!(
        oct.items,
        vec!["Compute: 2.00 EUR", "Block Storage: 0.30 EUR"]
    );
    assert_eq!(find(&r, "November 2026 to date").status, Status::Ok);
    assert_eq!(r.exit_code(), 2);
}

#[test]
fn rounding_noise_is_not_a_charge() {
    let mut s = base();
    s.usage = Ok(vec![usage("2026-10-03", "Compute", "x", 0.001, 1.0)]);
    assert_eq!(find(&run(&s), "October 2026").status, Status::Ok);
}

#[test]
fn unreadable_usage_is_a_warning_not_a_pass() {
    let mut s = base();
    s.usage = Err("NotAuthorizedOrNotFound".into());
    let r = run(&s);
    assert_eq!(find(&r, "Billed cost").status, Status::Warn);
    assert!(find(&r, "Billed cost").detail.contains("could not check"));
    assert_eq!(r.exit_code(), 1);
}

#[test]
fn egress_near_and_over_10_tb() {
    let mut s = base();
    s.usage = Ok(vec![usage(
        "2026-11-02",
        "Virtual Cloud Network",
        "Outbound Data Transfer Zone 1",
        0.0,
        9_000.0,
    )]);
    assert_eq!(
        find(&run(&s), "Outbound data this month").status,
        Status::Warn
    );
    s.usage = Ok(vec![usage(
        "2026-11-02",
        "Virtual Cloud Network",
        "Outbound Data Transfer Zone 1",
        0.0,
        11_000.0,
    )]);
    assert_eq!(
        find(&run(&s), "Outbound data this month").status,
        Status::Fail
    );
    // Last month's traffic does not count against this month.
    s.usage = Ok(vec![usage(
        "2026-10-02",
        "Virtual Cloud Network",
        "Outbound Data Transfer Zone 1",
        0.0,
        11_000.0,
    )]);
    assert_eq!(
        find(&run(&s), "Outbound data this month").status,
        Status::Ok
    );
}

#[test]
fn paid_shapes_fail_when_running_and_warn_when_stopped() {
    let mut s = base();
    s.instances = Ok(vec![
        inst("big", "VM.Standard.E4.Flex", "RUNNING", 1.0, 16.0),
        inst("parked", "VM.Standard3.Flex", "STOPPED", 1.0, 16.0),
        inst("gone", "VM.Standard3.Flex", "TERMINATED", 1.0, 16.0),
    ]);
    let r = run(&s);
    assert_eq!(find(&r, "big").status, Status::Fail);
    assert_eq!(find(&r, "parked").status, Status::Warn);
    assert!(
        r.sections
            .iter()
            .flat_map(|s| &s.findings)
            .all(|f| f.label != "gone")
    );
}

fn a1(s: &mut Snapshot, ocpus: f64, mem: f64) {
    s.instances = Ok(vec![inst(
        "arm",
        "VM.Standard.A1.Flex",
        "RUNNING",
        ocpus,
        mem,
    )]);
}

#[test]
fn a1_between_2_12_and_4_24_is_judged_by_the_bill() {
    let mut s = base();
    a1(&mut s, 4.0, 24.0);
    // Billed nothing for A1: free on this account.
    let f = find(&run(&s), "A1.Flex total").clone();
    assert_eq!(f.status, Status::Ok, "{f:?}");
    assert!(f.detail.contains("free on this account"));

    // Billed for A1.
    s.usage = Ok(vec![usage(
        "2026-10-05",
        "Compute",
        "Standard - A1 - Memory",
        0.50,
        100.0,
    )]);
    assert_eq!(find(&run(&s), "A1.Flex total").status, Status::Fail);

    // No A1 rows at all: cannot tell.
    s.usage = Ok(vec![]);
    assert_eq!(find(&run(&s), "A1.Flex total").status, Status::Warn);
    s.usage = Err("denied".into());
    assert_eq!(find(&run(&s), "A1.Flex total").status, Status::Warn);
}

#[test]
fn a1_above_4_24_always_fails_unless_the_user_says_otherwise() {
    let mut s = base();
    a1(&mut s, 6.0, 36.0);
    assert_eq!(find(&run(&s), "A1.Flex total").status, Status::Fail);
    let opts = Options {
        a1_limit: Allowance::parse("8/48"),
    };
    assert_eq!(find(&audit(&s, &opts), "A1.Flex total").status, Status::Ok);
    let opts = Options {
        a1_limit: Allowance::parse("4/24"),
    };
    assert_eq!(
        find(&audit(&s, &opts), "A1.Flex total").status,
        Status::Fail
    );
}

#[test]
fn stopped_a1_does_not_count() {
    let mut s = base();
    s.instances = Ok(vec![inst(
        "arm",
        "VM.Standard.A1.Flex",
        "STOPPED",
        4.0,
        24.0,
    )]);
    assert_eq!(find(&run(&s), "A1.Flex total").detail, "none running");
}

#[test]
fn more_than_two_micros_fail() {
    let mut s = base();
    s.instances = Ok((0..3)
        .map(|i| {
            inst(
                &format!("m{i}"),
                "VM.Standard.E2.1.Micro",
                "RUNNING",
                1.0,
                1.0,
            )
        })
        .collect());
    let f = find(&run(&s), "E2.1.Micro").clone();
    assert_eq!((f.status, f.detail.as_str()), (Status::Fail, "3 of 2"));
}

#[test]
fn storage_limits() {
    let mut s = base();
    s.block_volumes = Ok(vec![vol(85.0)]);
    assert_eq!(find(&run(&s), "Block volumes").status, Status::Warn); // 185 of 200
    s.block_volumes = Ok(vec![vol(150.0)]);
    assert_eq!(find(&run(&s), "Block volumes").status, Status::Fail); // 250
    s.block_volumes = Err("denied".into());
    assert_eq!(find(&run(&s), "Block volumes").status, Status::Warn);

    s.backups = Ok(vec![backup(); 5]);
    let f = find(&run(&s), "Volume backups").clone();
    assert_eq!(
        (f.status, f.detail.as_str()),
        (Status::Warn, "5 of 5 — no room left")
    );
    s.backups = Ok(vec![backup(); 6]);
    assert_eq!(find(&run(&s), "Volume backups").status, Status::Fail);
}

#[test]
fn budgets_without_alerts_are_not_a_safety_net() {
    let mut s = base();
    s.budgets = Ok(vec![]);
    assert_eq!(find(&run(&s), "Budget alert").status, Status::Warn);
    s.budgets = Ok(vec![Budget {
        display_name: "b".into(),
        amount: Some(1.0),
        alert_rule_count: Some(0),
    }]);
    let f = find(&run(&s), "Budget alert").clone();
    assert_eq!(f.status, Status::Warn);
    assert!(f.items[0].contains("cloud.oracle.com/usage/budgets"));
}

#[test]
fn other_regions_are_flagged_until_they_are_checked() {
    let mut s = base();
    s.regions.push("us-ashburn-1".into());
    let f = find(&run(&s), "Other regions").clone();
    assert_eq!(f.status, Status::Warn);
    assert_eq!(f.items, vec!["us-ashburn-1"]);
}

#[test]
fn names_hidden_by_inspect_only_access_fall_back_to_the_ocid() {
    let mut s = base();
    let mut i = inst("x", "VM.Standard.E4.Flex", "RUNNING", 1.0, 16.0);
    i.display_name = None;
    i.id = "ocid1.instance.oc1.eu-amsterdam-1.anabcdlprq".into();
    s.instances = Ok(vec![i]);
    assert_eq!(find(&run(&s), "instance …lprq").status, Status::Fail);
}
