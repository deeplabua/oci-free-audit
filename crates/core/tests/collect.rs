//! `api::collect` against a local mock of the OCI API: routes, pagination, a refused service.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use oci_free_audit_core::model::Snapshot;
use oci_free_audit_core::{Date, Options, Status, api, audit};
use oci_request_signer::{Client, Profile};

const T: &str = "ocid1.tenancy.oc1..t";

/// Answer for `method path` (path with query).
fn route(method: &str, path: &str) -> (u16, Vec<(&'static str, &'static str)>, String) {
    let ok = |body: &str| (200, vec![], body.to_owned());
    let p = path.split('?').next().unwrap();
    match (method, p) {
        ("GET", "/20160918/tenancies/ocid1.tenancy.oc1..t") => ok(r#"{"name":"acme","homeRegionKey":"AMS"}"#),
        ("GET", "/20160918/tenancies/ocid1.tenancy.oc1..t/regionSubscriptions") => ok(
            r#"[{"regionName":"eu-amsterdam-1","isHomeRegion":true,"status":"READY"}]"#,
        ),
        ("GET", "/20160918/compartments") => ok(r#"[{"id":"ocid1.compartment.oc1..c1","name":"apps"}]"#),
        ("GET", "/20160918/instances") if path.contains("ocid1.compartment.oc1..c1") => ok("[]"),
        // The root compartment's instances come in two pages.
        ("GET", "/20160918/instances") if path.contains("page=p2") => ok(
            r#"[{"id":"ocid1.instance.oc1..micro","displayName":"micro","shape":"VM.Standard.E2.1.Micro",
                 "lifecycleState":"RUNNING","shapeConfig":{"ocpus":1.0,"memoryInGBs":1.0}}]"#,
        ),
        ("GET", "/20160918/instances") => (
            200,
            vec![("opc-next-page", "p2")],
            r#"[{"id":"ocid1.instance.oc1..armlprq","displayName":null,"shape":"VM.Standard.A1.Flex",
                 "lifecycleState":"RUNNING","shapeConfig":{"ocpus":4.0,"memoryInGBs":24.0},"imageId":null}]"#
                .to_owned(),
        ),
        ("GET", "/20160918/volumes") => ok("[]"),
        ("GET", "/20160918/availabilityDomains") => ok(r#"[{"name":"Abcd:EU-AMSTERDAM-1-AD-1"}]"#),
        ("GET", "/20160918/bootVolumes") if path.contains("ocid1.compartment.oc1..c1") => ok("[]"),
        ("GET", "/20160918/bootVolumes") => ok(
            r#"[{"id":"ocid1.bootvolume.oc1..a","displayName":null,"sizeInGBs":47,"lifecycleState":"AVAILABLE"},
                {"id":"ocid1.bootvolume.oc1..b","displayName":null,"sizeInGBs":47,"lifecycleState":"AVAILABLE"}]"#,
        ),
        ("GET", "/20160918/bootVolumeBackups") | ("GET", "/20160918/volumeBackups") => ok("[]"),
        ("POST", "/20200107/usage") => ok(
            r#"{"items":[
              {"timeUsageStarted":"2026-10-05T00:00:00.000Z","service":"Compute","skuName":"Standard - A1",
               "computedAmount":0,"computedQuantity":96,"currency":"EUR","unit":"OCPU Hours"},
              {"timeUsageStarted":"2026-11-09T00:00:00.000Z","service":"Compute","skuName":"Standard - A1 - Memory",
               "computedAmount":0,"computedQuantity":576,"currency":"EUR","unit":"GB Hours"}]}"#,
        ),
        ("GET", "/20190111/budgets") => (
            404,
            vec![("opc-request-id", "req-b")],
            r#"{"code":"NotAuthorizedOrNotFound","message":"Authorization failed"}"#.to_owned(),
        ),
        ("GET", "/20160918/autonomousDatabases") if !path.contains("..c1") => ok(
            r#"[{"id":"ocid1.autonomousdatabase.oc1..a","displayName":"free-adb","isFreeTier":true,"lifecycleState":"AVAILABLE"}]"#,
        ),
        ("GET", "/20190415/dbSystems") | ("GET", "/20170115/loadBalancers") | ("GET", "/20180608/vaults") => ok("[]"),
        ("GET", "/20200501/networkLoadBalancers") => ok(r#"{"items":[]}"#),
        ("GET", "/20160918/vcns") if !path.contains("..c1") => ok(
            r#"[{"id":"ocid1.vcn.oc1..v","displayName":"vcn","lifecycleState":"AVAILABLE"}]"#,
        ),
        ("GET", "/20160918/autonomousDatabases") | ("GET", "/20160918/vcns") => ok("[]"),
        ("GET", "/n/") => ok(r#""acmens""#),
        ("GET", "/n/acmens/b/") if !path.contains("..c1") => ok(r#"[{"name":"photos","namespace":"acmens"}]"#),
        ("GET", "/n/acmens/b/") => ok("[]"),
        ("GET", "/n/acmens/b/photos") => ok(
            r#"{"name":"photos","storageTier":"Standard","approximateSize":3000000000}"#,
        ),
        ("GET", "/20230401/subscriptions") => ok(r#"{"items":[{"paymentModel":"PAYG","serviceName":"CLOUDCM"}]}"#),
        _ => (404, vec![], format!(r#"{{"code":"NotFound","message":"no mock for {method} {path}"}}"#)),
    }
}

/// Serve `route` until the test ends; return the base URL and the requests seen.
fn mock() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (method, path) = (
                parts.next().unwrap().to_owned(),
                parts.next().unwrap().to_owned(),
            );
            let mut len = 0;
            let mut signed = false;
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                let h = h.trim_end();
                if h.is_empty() {
                    break;
                }
                let lower = h.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                signed |= lower.starts_with("authorization: signature");
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            assert!(signed, "unsigned request {method} {path}");
            log.lock().unwrap().push(format!("{method} {path}"));
            let (status, headers, body) = route(&method, &path);
            let mut out = stream;
            let mut resp = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
                body.len()
            );
            for (k, v) in headers {
                resp += &format!("{k}: {v}\r\n");
            }
            resp += "\r\n";
            resp += &body;
            let _ = out.write_all(resp.as_bytes());
        }
    });
    (base, seen)
}

fn snapshot() -> (Snapshot, Vec<String>) {
    let (base, seen) = mock();
    let profile = Profile {
        user: "ocid1.user.oc1..u".into(),
        fingerprint: "aa:bb".into(),
        tenancy: T.into(),
        region: "eu-amsterdam-1".into(),
        key_file: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-key.pem"),
        pass_phrase: None,
    };
    let client = Client::builder(&profile).endpoint(base).build().unwrap();
    let snap = api::collect(
        &client,
        Date::parse("2026-10-01").unwrap(),
        Date::parse("2026-11-10").unwrap(),
    )
    .unwrap();
    let seen = seen.lock().unwrap().clone();
    (snap, seen)
}

#[test]
fn collects_everything_and_keeps_going_when_one_service_refuses() {
    let (s, seen) = snapshot();
    assert_eq!(s.tenancy_name, "acme");
    assert_eq!(s.home_region, "eu-amsterdam-1");
    assert_eq!(s.payment_model.as_deref(), Some("PAYG"));

    // Both pages of the root compartment, plus the child compartment.
    let instances = s.instances.as_ref().unwrap();
    assert_eq!(instances.len(), 2);
    assert_eq!(instances[0].name(), "instance …lprq");
    assert_eq!(instances[0].memory_gb(), 24.0);
    assert!(
        seen.iter()
            .any(|r| r.contains("/instances?") && r.contains("page=p2"))
    );
    assert!(
        seen.iter()
            .any(|r| r.contains("compartmentId=ocid1.compartment.oc1..c1"))
    );

    assert_eq!(s.boot_volumes.as_ref().unwrap().len(), 2);
    assert_eq!(s.boot_volumes.as_ref().unwrap()[0].region, "eu-amsterdam-1");
    assert_eq!(s.autonomous_dbs.as_ref().unwrap().len(), 1);
    assert_eq!(s.vcns.as_ref().unwrap().len(), 1);
    let buckets = s.buckets.as_ref().unwrap();
    assert_eq!(
        (buckets[0].name.as_str(), buckets[0].approximate_size),
        ("photos", Some(3e9))
    );
    assert_eq!(s.usage.as_ref().unwrap().len(), 2);
    let budgets = s.budgets.as_ref().unwrap_err();
    assert!(budgets.contains("NotAuthorizedOrNotFound"), "{budgets}");

    // The query the Usage API got.
    assert!(seen.contains(&"POST /20200107/usage".to_owned()));
    // Nothing but reads.
    assert!(
        seen.iter()
            .all(|r| r.starts_with("GET ") || r == "POST /20200107/usage"),
        "{seen:#?}"
    );
}

#[test]
fn the_report_for_the_mock_account() {
    let (s, _) = snapshot();
    let r = audit(&s, &Options::default());
    let all: Vec<_> = r.sections.iter().flat_map(|s| &s.findings).collect();
    let a1 = all.iter().find(|f| f.label == "A1.Flex total").unwrap();
    assert_eq!(a1.status, Status::Ok, "{a1:?}"); // 4/24, billed 0
    let storage = all.iter().find(|f| f.label == "Block volumes").unwrap();
    assert_eq!(storage.detail, "94 GB of 200 (boot 94, block 0)");
    let budget = all.iter().find(|f| f.label == "Budget alert").unwrap();
    assert_eq!(budget.status, Status::Warn);
    let os = all.iter().find(|f| f.label == "Object Storage").unwrap();
    assert_eq!(
        os.detail,
        "Standard 3.0 GB of 10, Archive 0.0 GB of 10 (1 buckets)"
    );
    let adb = all
        .iter()
        .find(|f| f.label == "Autonomous Database (free tier)")
        .unwrap();
    assert_eq!(adb.detail, "1 of 2");
    assert_eq!(r.exit_code(), 1);
}
