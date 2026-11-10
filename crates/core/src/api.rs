//! Reads the tenancy — GET requests and read-only Usage API queries, nothing else.

use oci_request_signer::{Client, Method, OCI_DOMAIN, Result, encode};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::date::Date;
use crate::model::{Backup, Budget, Listed, Snapshot, UsageItem, Volume};

const IDENTITY: &str = "/20160918";
const CORE: &str = "/20160918";

/// Read everything the checks need. Fails only if the tenancy itself cannot be read
/// (config, key, network); a list OCI refuses is kept as an error in the snapshot.
pub fn collect(client: &Client, since: Date, today: Date) -> Result<Snapshot> {
    let tenancy_id = client.tenancy().to_owned();
    let tenancy: Value = client.get("identity", &format!("{IDENTITY}/tenancies/{tenancy_id}"))?;
    let regions: Vec<Value> = client.get(
        "identity",
        &format!("{IDENTITY}/tenancies/{tenancy_id}/regionSubscriptions"),
    )?;
    let home_region = regions
        .iter()
        .find(|r| r["isHomeRegion"] == true)
        .and_then(|r| r["regionName"].as_str())
        .unwrap_or(client.region())
        .to_owned();
    let home = client.in_region(home_region.clone());
    let oci = home.on_domain(OCI_DOMAIN);
    let t = encode(&tenancy_id);

    let compartments = compartments(&home, &tenancy_id);
    let per_compartment = |path: &dyn Fn(&str) -> String| -> Listed<Value> {
        let ids = compartments.as_ref().map_err(Clone::clone)?;
        let mut all = Vec::new();
        for id in ids {
            all.extend(
                list_all::<Value>(&home, "iaas", &path(&encode(id))).map_err(|e| e.to_string())?,
            );
        }
        Ok(all)
    };

    let instances =
        per_compartment(&|c| format!("{CORE}/instances?compartmentId={c}")).and_then(parse);
    let block_volumes =
        per_compartment(&|c| format!("{CORE}/volumes?compartmentId={c}")).and_then(parse);
    let boot_volumes = list_all::<Value>(
        &home,
        "identity",
        &format!("{IDENTITY}/availabilityDomains?compartmentId={t}"),
    )
    .map_err(|e| e.to_string())
    .and_then(|ads| {
        let names: Vec<String> = ads
            .iter()
            .filter_map(|a| a["name"].as_str().map(str::to_owned))
            .collect();
        let mut all = Vec::new();
        for ad in &names {
            let ad = encode(ad);
            all.extend(per_compartment(&|c| {
                format!("{CORE}/bootVolumes?compartmentId={c}&availabilityDomain={ad}")
            })?);
        }
        Ok(all)
    })
    .and_then(parse::<Volume>);
    let backups = per_compartment(&|c| format!("{CORE}/bootVolumeBackups?compartmentId={c}"))
        .and_then(|mut boot| {
            boot.extend(per_compartment(&|c| {
                format!("{CORE}/volumeBackups?compartmentId={c}")
            })?);
            Ok(boot)
        })
        .and_then(parse::<Backup>);

    let usage = usage(&oci, &tenancy_id, since, today);
    let budgets = list_all::<Budget>(
        &oci,
        "usage",
        &format!("/20190111/budgets?compartmentId={t}&targetType=ALL"),
    )
    .map_err(|e| e.to_string());
    let payment_model = list_all_items(
        &oci,
        "organizations",
        &format!("/20230401/subscriptions?compartmentId={t}"),
    )
    .ok()
    .and_then(|subs| {
        subs.iter()
            .find_map(|s| s["paymentModel"].as_str().map(str::to_owned))
    });

    Ok(Snapshot {
        tenancy_name: tenancy["name"].as_str().unwrap_or("").to_owned(),
        tenancy_id,
        home_region,
        regions: regions
            .iter()
            .filter_map(|r| r["regionName"].as_str().map(str::to_owned))
            .collect(),
        payment_model,
        instances,
        boot_volumes,
        block_volumes,
        backups,
        usage,
        budgets,
        today,
        since,
    })
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
