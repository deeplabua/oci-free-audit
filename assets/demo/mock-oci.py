#!/usr/bin/env python3
"""A tiny stand-in for the OCI API, for recording the README GIF.

Answers the calls `oci-free-audit` makes for a made-up PAYG tenancy: a 4/24 A1 billed 0.00,
an E2.1.Micro, two boot volumes, one bucket, an Always Free Autonomous Database — and no budget.
It does not check signatures. Not for anything but demos.

    python3 assets/demo/mock-oci.py 8797
"""

import datetime
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

REGION = "eu-amsterdam-1"
T = "ocid1.tenancy.oc1..aaaademotenancy"


def day_rows():
    rows = []
    for month, days in (("10", 31), ("11", 9)):
        for d in range(1, days + 1):
            day = f"2026-{month}-{d:02d}T00:00:00.000Z"
            for svc, sku, qty in (("Compute", "Standard - A1", 96), ("Compute", "Standard - A1 - Memory", 576),
                                  ("Compute", "Virtual Machine Standard - E2 Micro - Free", 24),
                                  ("Virtual Cloud Network", "Outbound Data Transfer Zone 1", 0.4)):
                rows.append({"timeUsageStarted": day, "service": svc, "skuName": sku, "computedAmount": 0,
                             "computedQuantity": qty, "currency": "EUR"})
    return rows


GET = {
    f"/20160918/tenancies/{T}": {"name": "acme-xyz"},
    f"/20160918/tenancies/{T}/regionSubscriptions": [{"regionName": REGION, "isHomeRegion": True}],
    "/20160918/compartments": [],
    "/20160918/availabilityDomains": [{"name": "Abcd:EU-AMSTERDAM-1-AD-1"}],
    "/20160918/instances": [
        {"id": "ocid1.instance.oc1..arm", "displayName": "arm-free-1", "shape": "VM.Standard.A1.Flex",
         "lifecycleState": "RUNNING", "shapeConfig": {"ocpus": 4, "memoryInGBs": 24}},
        {"id": "ocid1.instance.oc1..micro", "displayName": "micro-1", "shape": "VM.Standard.E2.1.Micro",
         "lifecycleState": "RUNNING", "shapeConfig": {"ocpus": 1, "memoryInGBs": 1}},
    ],
    "/20160918/bootVolumes": [
        {"id": "ocid1.bootvolume.oc1..a", "displayName": "arm-free-1 (Boot Volume)", "sizeInGBs": 100, "lifecycleState": "AVAILABLE"},
        {"id": "ocid1.bootvolume.oc1..b", "displayName": "micro-1 (Boot Volume)", "sizeInGBs": 47, "lifecycleState": "AVAILABLE"},
    ],
    "/20160918/volumes": [],
    "/20160918/bootVolumeBackups": [{"id": "ocid1.bootvolumebackup.oc1..a", "displayName": "weekly", "lifecycleState": "AVAILABLE"}],
    "/20160918/volumeBackups": [],
    "/20160918/autonomousDatabases": [{"id": "ocid1.autonomousdatabase.oc1..a", "displayName": "notes-db",
                                       "isFreeTier": True, "lifecycleState": "AVAILABLE"}],
    "/20190415/dbSystems": [],
    "/20170115/loadBalancers": [],
    "/20200501/networkLoadBalancers": {"items": []},
    "/20160918/vcns": [{"id": "ocid1.vcn.oc1..a", "displayName": "main", "lifecycleState": "AVAILABLE"}],
    "/20180608/vaults": [],
    "/n/": "acmens",
    "/n/acmens/b/": [{"name": "backups"}],
    "/n/acmens/b/backups": {"name": "backups", "storageTier": "Standard", "approximateSize": 1_800_000_000},
    "/20190111/budgets": [],
    # MOCK_TRIAL=1: a new account, 30-day trial ending in 12 days.
    "/20230401/subscriptions": {"items": [
        {"paymentModel": "FREE_TIER",
         "startDate": (datetime.date.today() - datetime.timedelta(days=18)).isoformat() + "T00:00:00.000Z",
         "endDate": (datetime.date.today() + datetime.timedelta(days=12)).isoformat() + "T23:59:59.000Z"}
    ] if os.environ.get("MOCK_TRIAL") else [{"paymentModel": "PAYG"}]},
}


class Handler(BaseHTTPRequestHandler):
    def reply(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        path = self.path.split("?")[0]
        if path in GET:
            self.reply(200, GET[path])
        else:
            self.reply(404, {"code": "NotAuthorizedOrNotFound", "message": path})

    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length", 0)))
        self.reply(200, {"items": day_rows()})

    def log_message(self, *args):
        pass


HTTPServer(("127.0.0.1", int(sys.argv[1]) if len(sys.argv) > 1 else 8797), Handler).serve_forever()
