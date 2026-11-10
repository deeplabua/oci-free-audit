//! Every rule of the audit on hand-built snapshots.

use oci_free_audit_core::limits::Allowance;
use oci_free_audit_core::model::{
    Backup, Bucket, Budget, Instance, LbShape, LoadBalancer, Service, ShapeConfig, Snapshot,
    UsageItem, Volume,
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
        region: String::new(),
    }
}

fn vol(gb: f64) -> Volume {
    Volume {
        id: "ocid1.volume.oc1..v".into(),
        display_name: None,
        size_in_gbs: Some(gb),
        lifecycle_state: "AVAILABLE".into(),
        region: String::new(),
    }
}

fn backup() -> Backup {
    Backup {
        id: "ocid1.backup.oc1..b".into(),
        display_name: None,
        lifecycle_state: "AVAILABLE".into(),
        region: String::new(),
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
        trial_end: None,
        instances: Ok(vec![
            inst("micro", "VM.Standard.E2.1.Micro", "RUNNING", 1.0, 1.0),
            inst("arm", "VM.Standard.A1.Flex", "RUNNING", 2.0, 12.0),
        ]),
        boot_volumes: Ok(vec![vol(50.0), vol(50.0)]),
        block_volumes: Ok(vec![]),
        backups: Ok(vec![backup()]),
        buckets: Ok(vec![]),
        autonomous_dbs: Ok(vec![]),
        mysql: Ok(vec![]),
        load_balancers: Ok(vec![]),
        network_load_balancers: Ok(vec![]),
        vcns: Ok(vec![svc("vcn")]),
        vaults: Ok(vec![]),
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
fn names_hidden_by_inspect_only_access_fall_back_to_the_ocid() {
    let mut s = base();
    let mut i = inst("x", "VM.Standard.E4.Flex", "RUNNING", 1.0, 16.0);
    i.display_name = None;
    i.id = "ocid1.instance.oc1.eu-amsterdam-1.anabcdlprq".into();
    s.instances = Ok(vec![i]);
    assert_eq!(find(&run(&s), "instance …lprq").status, Status::Fail);
}

fn svc(name: &str) -> Service {
    Service {
        id: format!("ocid1.x.oc1..{name}"),
        display_name: Some(name.into()),
        lifecycle_state: Some("AVAILABLE".into()),
        is_free_tier: None,
        shape_name: None,
        vault_type: None,
        region: "eu-amsterdam-1".into(),
    }
}

#[test]
fn compute_and_volumes_outside_the_home_region_are_billed() {
    let mut s = base();
    let mut far = inst("far", "VM.Standard.A1.Flex", "RUNNING", 1.0, 6.0);
    far.region = "us-ashburn-1".into();
    let mut parked = inst("parked", "VM.Standard.E2.1.Micro", "STOPPED", 1.0, 1.0);
    parked.region = "us-ashburn-1".into();
    let mut home = inst("arm", "VM.Standard.A1.Flex", "RUNNING", 2.0, 12.0);
    home.region = "eu-amsterdam-1".into();
    s.instances = Ok(vec![far, parked, home]);
    let mut v = vol(47.0);
    v.region = "us-ashburn-1".into();
    s.block_volumes = Ok(vec![v]);
    let r = run(&s);
    assert_eq!(find(&r, "far").status, Status::Fail);
    assert!(
        find(&r, "far")
            .detail
            .contains("free only in the home region")
    );
    assert_eq!(find(&r, "parked").status, Status::Warn);
    // The far A1 does not count towards the home allowance.
    assert_eq!(find(&r, "A1.Flex total").status, Status::Ok);
    let outside = find(&r, "Volumes outside the home region").clone();
    assert_eq!(
        (outside.status, outside.items.clone()),
        (Status::Fail, vec!["47 GB in us-ashburn-1".to_owned()])
    );
    assert_eq!(
        find(&r, "Block volumes").detail,
        "100 GB of 200 (boot 100, block 0)"
    );
}

fn bucket(name: &str, tier: &str, gb: Option<f64>) -> Bucket {
    Bucket {
        name: name.into(),
        storage_tier: Some(tier.into()),
        approximate_size: gb.map(|g| g * 1e9),
        region: "eu-amsterdam-1".into(),
    }
}

#[test]
fn object_storage_limits_depend_on_the_account() {
    let mut s = base();
    assert_eq!(find(&run(&s), "Object Storage").detail, "no buckets");
    // PAYG: 10 GB each of Standard and Archive.
    s.buckets = Ok(vec![
        bucket("a", "Standard", Some(9.5)),
        bucket("b", "Archive", Some(2.0)),
    ]);
    assert_eq!(find(&run(&s), "Object Storage").status, Status::Warn);
    s.buckets = Ok(vec![bucket("a", "Standard", Some(12.0))]);
    assert_eq!(find(&run(&s), "Object Storage").status, Status::Fail);
    // Always Free only: 20 GB all together.
    s.payment_model = None;
    s.buckets = Ok(vec![
        bucket("a", "Standard", Some(12.0)),
        bucket("b", "Archive", Some(5.0)),
    ]);
    let f = find(&run(&s), "Object Storage").clone();
    assert_eq!(
        (f.status, f.detail.as_str()),
        (Status::Ok, "17.0 GB of 20 (2 buckets)")
    );
    // Size refused: say why.
    s.buckets = Ok(vec![bucket("a", "Standard", None)]);
    let f = find(&run(&s), "Object Storage").clone();
    assert_eq!(f.status, Status::Warn);
    assert!(f.detail.contains("read buckets"));
}

#[test]
fn autonomous_databases_and_mysql() {
    let mut s = base();
    let free = |n: &str| Service {
        is_free_tier: Some(true),
        ..svc(n)
    };
    s.autonomous_dbs = Ok(vec![free("a"), free("b")]);
    assert_eq!(
        find(&run(&s), "Autonomous Database (free tier)").status,
        Status::Ok
    );
    let paid = Service {
        is_free_tier: Some(false),
        ..svc("paid")
    };
    let stopped = Service {
        is_free_tier: Some(false),
        lifecycle_state: Some("STOPPED".into()),
        ..svc("parked")
    };
    let gone = Service {
        is_free_tier: Some(false),
        lifecycle_state: Some("TERMINATED".into()),
        ..svc("gone")
    };
    s.autonomous_dbs = Ok(vec![free("a"), paid, stopped, gone]);
    let r = run(&s);
    assert_eq!(find(&r, "paid").status, Status::Fail);
    assert_eq!(find(&r, "parked").status, Status::Warn);
    assert!(
        r.sections
            .iter()
            .flat_map(|s| &s.findings)
            .all(|f| f.label != "gone")
    );

    s.mysql = Ok(vec![Service {
        shape_name: Some("MySQL.Free".into()),
        lifecycle_state: Some("ACTIVE".into()),
        ..svc("free-db")
    }]);
    assert_eq!(find(&run(&s), "MySQL HeatWave (free)").detail, "1 of 1");
    s.mysql = Ok(vec![Service {
        shape_name: Some("MySQL.2".into()),
        lifecycle_state: Some("ACTIVE".into()),
        ..svc("big-db")
    }]);
    assert_eq!(find(&run(&s), "big-db").status, Status::Fail);
}

#[test]
fn load_balancers_nlbs_and_vcns() {
    let lb = |name: &str, shape: &str, max: Option<f64>| LoadBalancer {
        id: format!("ocid1.loadbalancer.oc1..{name}"),
        display_name: Some(name.into()),
        shape_name: Some(shape.into()),
        shape_details: max.map(|m| LbShape {
            minimum_bandwidth_in_mbps: Some(10.0),
            maximum_bandwidth_in_mbps: Some(m),
        }),
        lifecycle_state: Some("ACTIVE".into()),
        region: "eu-amsterdam-1".into(),
    };
    let mut s = base();
    s.load_balancers = Ok(vec![lb("free", "flexible", Some(10.0))]);
    assert_eq!(
        find(&run(&s), "Load balancer (10 Mbit/s)").status,
        Status::Ok
    );
    s.load_balancers = Ok(vec![
        lb("free", "flexible", Some(10.0)),
        lb("fast", "flexible", Some(100.0)),
        lb("old", "100Mbps", None),
    ]);
    let r = run(&s);
    assert_eq!(find(&r, "fast").status, Status::Fail);
    assert_eq!(find(&r, "old").status, Status::Fail);
    s.load_balancers = Ok(vec![
        lb("a", "flexible", Some(10.0)),
        lb("b", "10Mbps-Micro", None),
    ]);
    assert_eq!(
        find(&run(&s), "Load balancer (10 Mbit/s)").status,
        Status::Fail
    );

    s.network_load_balancers = Ok(vec![svc("n1"), svc("n2")]);
    assert_eq!(
        find(&run(&s), "Network load balancers").status,
        Status::Warn
    );

    s.vcns = Ok(vec![svc("v1"), svc("v2"), svc("v3")]);
    assert_eq!(find(&run(&s), "VCNs").status, Status::Ok, "no cap on PAYG");
    s.payment_model = None;
    assert_eq!(find(&run(&s), "VCNs").status, Status::Warn);
}

#[test]
fn private_vaults_are_billed() {
    let mut s = base();
    s.vaults = Ok(vec![Service {
        vault_type: Some("DEFAULT".into()),
        ..svc("keys")
    }]);
    assert_eq!(find(&run(&s), "Vaults").detail, "1 virtual (free)");
    s.vaults = Ok(vec![Service {
        vault_type: Some("VIRTUAL_PRIVATE".into()),
        ..svc("hsm")
    }]);
    let f = find(&run(&s), "Vaults").clone();
    assert_eq!(f.status, Status::Fail);
    assert_eq!(f.items, vec!["hsm in eu-amsterdam-1"]);
}

#[test]
fn trial_running_everything_free() {
    let mut s = base();
    s.payment_model = Some("FREE_TIER".into());
    s.trial_end = Some(d("2026-11-20"));
    let r = run(&s);
    assert_eq!(r.sections[0].title, "TRIAL");
    let f = &r.sections[0].findings[0];
    assert_eq!(f.status, Status::Ok);
    assert!(
        f.detail.contains("2026-11-20 (10 days left)"),
        "{}",
        f.detail
    );
    assert_eq!(r.trial_ends, Some(d("2026-11-20")));
}

#[test]
fn trial_running_with_paid_resources_and_cost() {
    let mut s = base();
    s.payment_model = Some("FREE_TIER".into());
    s.trial_end = Some(d("2026-11-11"));
    if let Ok(instances) = &mut s.instances {
        instances.push(inst("big", "VM.Standard.E4.Flex", "RUNNING", 2.0, 16.0));
    }
    s.usage = Ok(vec![usage("2026-11-05", "Compute", "E4 OCPU", 3.5, 48.0)]);
    let r = run(&s);
    let trial = &r.sections[0].findings[0];
    assert_eq!(trial.status, Status::Warn);
    assert!(trial.detail.contains("(1 day left)") && trial.detail.contains("stopped then"));
    assert!(
        trial.items.iter().any(|i| i.contains("big")),
        "{:?}",
        trial.items
    );
    // Cost goes to the trial credits: ⚠, not ✗.
    let cost = r.sections.iter().find(|x| x.title == "COST").unwrap();
    assert!(cost.findings.iter().all(|f| f.status != Status::Fail));
    assert!(
        cost.findings
            .iter()
            .any(|f| f.detail.contains("trial credits"))
    );
}

#[test]
fn trial_over_is_not_shown() {
    let mut s = base();
    s.trial_end = Some(d("2026-07-03"));
    let r = run(&s);
    assert!(r.sections.iter().all(|x| x.title != "TRIAL"));
    assert_eq!(r.trial_ends, None);
}
