//! Reads the tenancy — GET requests and read-only Usage API queries, nothing else.

use oci_request_signer::{Client, Method, OCI_DOMAIN, Result, encode};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::date::Date;
use crate::limits;
use crate::model::{
    Backup, Bucket, Budget, Instance, Listed, LoadBalancer, Service, Snapshot, UsageItem, Volume,
};

const IDENTITY: &str = "/20160918";
const CORE: &str = "/20160918";

/// Read everything the checks need. Fails only if the tenancy itself cannot be read
/// (config, key, network); a list OCI refuses is kept as an error in the snapshot.
pub fn collect(client: &Client, since: Date, today: Date) -> Result<Snapshot> {
    let tenancy_id = client.tenancy().to_owned();
    let tenancy: Value = client.get("identity", &format!("{IDENTITY}/tenancies/{tenancy_id}"))?;
    let subscriptions: Vec<Value> = client.get(
        "identity",
        &format!("{IDENTITY}/tenancies/{tenancy_id}/regionSubscriptions"),
    )?;
    let home_region = subscriptions
        .iter()
        .find(|r| r["isHomeRegion"] == true)
        .and_then(|r| r["regionName"].as_str())
        .unwrap_or(client.region())
        .to_owned();
    let mut regions: Vec<String> = subscriptions
        .iter()
        .filter_map(|r| r["regionName"].as_str().map(str::to_owned))
        .collect();
    if regions.is_empty() {
        regions.push(home_region.clone());
    }
    let home = client.in_region(home_region.clone());
    let oci = home.on_domain(OCI_DOMAIN);
    let t = encode(&tenancy_id);
    let compartments = compartments(&home, &tenancy_id);

    // GET `path(compartment)` in every region and compartment; each item tagged with its region.
    let everywhere = |service: &str,
                      domain: Option<&str>,
                      wrapped: bool,
                      path: &dyn Fn(&str) -> String|
     -> Listed<(String, Value)> {
        let ids = compartments.as_ref().map_err(Clone::clone)?;
        let mut all = Vec::new();
        for region in &regions {
            let mut c = home.in_region(region.clone());
            if let Some(d) = domain {
                c = c.on_domain(d);
            }
            for id in ids {
                let p = path(&encode(id));
                let list = if wrapped {
                    list_all_items(&c, service, &p)
                } else {
                    list_all::<Value>(&c, service, &p)
                };
                all.extend(
                    list.map_err(|e| format!("{region}: {e}"))?
                        .into_iter()
                        .map(|v| (region.clone(), v)),
                );
            }
        }
        Ok(all)
    };

    let instances = everywhere("iaas", None, false, &|c| {
        format!("{CORE}/instances?compartmentId={c}")
    })
    .and_then(|v| tagged(v, |i: &mut Instance, r| i.region = r));
    let block_volumes = everywhere("iaas", None, false, &|c| {
        format!("{CORE}/volumes?compartmentId={c}")
    })
    .and_then(|v| tagged(v, |i: &mut Volume, r| i.region = r));
    let boot_volumes = boot_volumes(&home, &regions, &compartments, &t)
        .and_then(|v| tagged(v, |i: &mut Volume, r| i.region = r));
    let backups = everywhere("iaas", None, false, &|c| {
        format!("{CORE}/bootVolumeBackups?compartmentId={c}")
    })
    .and_then(|mut boot| {
        boot.extend(everywhere("iaas", None, false, &|c| {
            format!("{CORE}/volumeBackups?compartmentId={c}")
        })?);
        Ok(boot)
    })
    .and_then(|v| tagged(v, |i: &mut Backup, r| i.region = r));

    let set_region = |s: &mut Service, r: String| s.region = r;
    let autonomous_dbs = everywhere("database", None, false, &|c| {
        format!("{CORE}/autonomousDatabases?compartmentId={c}")
    })
    .and_then(|v| tagged(v, set_region));
    let mysql = everywhere("mysql", Some(OCP_DOMAIN), false, &|c| {
        format!("/20190415/dbSystems?compartmentId={c}")
    })
    .and_then(|v| tagged(v, set_region));
    let load_balancers = everywhere("iaas", None, false, &|c| {
        format!("/20170115/loadBalancers?compartmentId={c}")
    })
    .and_then(|v| tagged(v, |i: &mut LoadBalancer, r| i.region = r));
    let network_load_balancers =
        everywhere("network-load-balancer-api", Some(OCI_DOMAIN), true, &|c| {
            format!("/20200501/networkLoadBalancers?compartmentId={c}")
        })
        .and_then(|v| tagged(v, set_region));
    let vcns = everywhere("iaas", None, false, &|c| {
        format!("{CORE}/vcns?compartmentId={c}")
    })
    .and_then(|v| tagged(v, set_region));
    let vaults = everywhere("kms", None, false, &|c| {
        format!("/20180608/vaults?compartmentId={c}")
    })
    .and_then(|v| tagged(v, set_region));
    let buckets = buckets(&home, &regions, &compartments);

    let usage = usage(&oci, &tenancy_id, since, today);
    let budgets = list_all::<Budget>(
        &oci,
        "usage",
        &format!("/20190111/budgets?compartmentId={t}&targetType=ALL"),
    )
    .map_err(|e| e.to_string());
    let subscriptions = list_all_items(
        &oci,
        "organizations",
        &format!("/20230401/subscriptions?compartmentId={t}"),
    )
    .unwrap_or_default();
    let payment_model = subscriptions
        .iter()
        .find_map(|s| s["paymentModel"].as_str().map(str::to_owned));
    // Checked on a real account, 10 November 2026: startDate 2026-06-04, endDate 2026-07-03 —
    // the trial — and paymentModel PAYG after the upgrade.
    let trial_end = subscriptions
        .iter()
        .filter_map(|s| {
            let start = s["startDate"].as_str().and_then(Date::parse)?;
            let end = s["endDate"].as_str().and_then(Date::parse)?;
            (end.to_days() - start.to_days() <= limits::TRIAL_MAX_DAYS).then_some(end)
        })
        .max();

    Ok(Snapshot {
        tenancy_name: tenancy["name"].as_str().unwrap_or("").to_owned(),
        tenancy_id,
        home_region,
        regions,
        payment_model,
        trial_end,
        instances,
        boot_volumes,
        block_volumes,
        backups,
        buckets,
        autonomous_dbs,
        mysql,
        load_balancers,
        network_load_balancers,
        vcns,
        vaults,
        usage,
        budgets,
        today,
        since,
    })
}

/// The domain MySQL HeatWave answers on: `mysql.<region>.ocp.oraclecloud.com`.
const OCP_DOMAIN: &str = "ocp.oraclecloud.com";

/// Boot volumes are listed per availability domain.
fn boot_volumes(
    home: &Client,
    regions: &[String],
    compartments: &Listed<String>,
    t: &str,
) -> Listed<(String, Value)> {
    let ids = compartments.as_ref().map_err(Clone::clone)?;
    let mut all = Vec::new();
    for region in regions {
        let c = home.in_region(region.clone());
        let ads = list_all::<Value>(
            &c,
            "identity",
            &format!("{IDENTITY}/availabilityDomains?compartmentId={t}"),
        )
        .map_err(|e| format!("{region}: {e}"))?;
        for ad in ads.iter().filter_map(|a| a["name"].as_str()) {
            for id in ids {
                let path = format!(
                    "{CORE}/bootVolumes?compartmentId={}&availabilityDomain={}",
                    encode(id),
                    encode(ad)
                );
                let list =
                    list_all::<Value>(&c, "iaas", &path).map_err(|e| format!("{region}: {e}"))?;
                all.extend(list.into_iter().map(|v| (region.clone(), v)));
            }
        }
    }
    Ok(all)
}

/// Buckets with their size. `approximateSize` needs `read buckets`; without it the size stays
/// unknown and the check says so.
fn buckets(home: &Client, regions: &[String], compartments: &Listed<String>) -> Listed<Bucket> {
    let ids = compartments.as_ref().map_err(Clone::clone)?;
    let ns: String = home
        .get("objectstorage", "/n/")
        .map_err(|e| e.to_string())?;
    let mut all = Vec::new();
    for region in regions {
        let c = home.in_region(region.clone());
        for id in ids {
            let path = format!("/n/{ns}/b/?compartmentId={}", encode(id));
            for b in list_all::<Value>(&c, "objectstorage", &path)
                .map_err(|e| format!("{region}: {e}"))?
            {
                let Some(name) = b["name"].as_str() else {
                    continue;
                };
                let detail = c
                    .get::<Value>(
                        "objectstorage",
                        &format!("/n/{ns}/b/{}?fields=approximateSize", encode(name)),
                    )
                    .unwrap_or(b.clone());
                let mut bucket: Bucket = serde_json::from_value(detail)
                    .map_err(|e| format!("unexpected response: {e}"))?;
                bucket.region = region.clone();
                all.push(bucket);
            }
        }
    }
    Ok(all)
}

/// Parse region-tagged values and record the region in each item.
fn tagged<T: DeserializeOwned>(
    values: Vec<(String, Value)>,
    set: impl Fn(&mut T, String),
) -> Listed<T> {
    values
        .into_iter()
        .map(|(region, v)| {
            let mut item: T =
                serde_json::from_value(v).map_err(|e| format!("unexpected response: {e}"))?;
            set(&mut item, region);
            Ok(item)
        })
        .collect()
}

/// The root compartment and every active compartment under it.
fn compartments(client: &Client, tenancy: &str) -> Listed<String> {
    let path = format!(
        "{IDENTITY}/compartments?compartmentId={}&compartmentIdInSubtree=true&accessLevel=ACCESSIBLE&lifecycleState=ACTIVE",
        encode(tenancy)
    );
    let mut ids = vec![tenancy.to_owned()];
    for c in list_all::<Value>(client, "identity", &path).map_err(|e| e.to_string())? {
        if let Some(id) = c["id"].as_str() {
            ids.push(id.to_owned());
        }
    }
    Ok(ids)
}

/// Daily cost by service and SKU from `since` up to (not including) `today`.
fn usage(client: &Client, tenancy: &str, since: Date, today: Date) -> Listed<UsageItem> {
    let end = if today > since {
        today
    } else {
        Date::from_days(since.to_days() + 1)
    };
    let body = json!({
        "tenantId": tenancy,
        "granularity": "DAILY",
        "queryType": "COST",
        "timeUsageStarted": since.rfc3339(),
        "timeUsageEnded": end.rfc3339(),
        "groupBy": ["service", "skuName"],
    });
    let body = serde_json::to_vec(&body).expect("the query always serializes");
    let mut items = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let path = match &page {
            Some(p) => format!("/20200107/usage?page={}", encode(p)),
            None => "/20200107/usage".to_owned(),
        };
        let resp = client
            .request(Method::Post, "usageapi", &path, Some(&body), &[])
            .map_err(|e| e.to_string())?;
        let v: Value = resp.json().map_err(|e| e.to_string())?;
        items.extend(parse::<UsageItem>(
            v["items"].as_array().cloned().unwrap_or_default(),
        )?);
        match resp.header("opc-next-page") {
            Some(next) if !next.is_empty() => page = Some(next.to_owned()),
            _ => return Ok(items),
        }
    }
}

/// GET a list, following `opc-next-page`. The answer is a JSON array.
fn list_all<T: DeserializeOwned>(client: &Client, service: &str, path: &str) -> Result<Vec<T>> {
    pages(client, service, path, |v| {
        serde_json::from_value::<Vec<T>>(v)
    })
}

/// GET a list whose answer is `{"items": [...]}`.
fn list_all_items(client: &Client, service: &str, path: &str) -> Result<Vec<Value>> {
    pages(client, service, path, |v| {
        serde_json::from_value::<Vec<Value>>(
            v.get("items").cloned().unwrap_or(Value::Array(vec![])),
        )
    })
}

fn pages<T>(
    client: &Client,
    service: &str,
    path: &str,
    decode: impl Fn(Value) -> serde_json::Result<Vec<T>>,
) -> Result<Vec<T>> {
    let mut all = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let sep = if path.contains('?') { '&' } else { '?' };
        let url = match &page {
            Some(p) => format!("{path}{sep}page={}", encode(p)),
            None => path.to_owned(),
        };
        let resp = client.request(Method::Get, service, &url, None, &[])?;
        let v: Value = resp.json()?;
        all.extend(decode(v).map_err(|e| {
            oci_request_signer::Error::Transport(format!("unexpected response from {service}: {e}"))
        })?);
        match resp.header("opc-next-page") {
            Some(next) if !next.is_empty() => page = Some(next.to_owned()),
            _ => return Ok(all),
        }
    }
}

fn parse<T: DeserializeOwned>(values: Vec<Value>) -> Listed<T> {
    values
        .into_iter()
        .map(|v| serde_json::from_value(v).map_err(|e| format!("unexpected response: {e}")))
        .collect()
}
