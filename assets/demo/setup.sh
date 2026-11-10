#!/usr/bin/env bash
# Sourced (hidden) by demo.tape: a throwaway OCI profile, a mock OCI API on localhost,
# and a clean prompt. Nothing here touches a real account.
#
# Record from the repo root:
#   cargo build --release
#   vhs assets/demo/demo.tape

DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$DEMO_DIR/../.." && pwd)"
WORK="$(mktemp -d)"

cat > "$WORK/oci-config" <<CONF
[DEFAULT]
user=ocid1.user.oc1..aaaademo
fingerprint=12:34:56:78:9a:bc:de:f0:12:34:56:78:9a:bc:de:f0
tenancy=ocid1.tenancy.oc1..aaaademotenancy
region=eu-amsterdam-1
key_file=$REPO/crates/core/tests/fixtures/test-key.pem
CONF

python3 "$DEMO_DIR/mock-oci.py" 8797 &
MOCK_PID=$!
trap 'kill $MOCK_PID 2>/dev/null; rm -rf "$WORK"' EXIT

export OCI_CONFIG_FILE="$WORK/oci-config"
export OCI_FREE_AUDIT_ENDPOINT="http://127.0.0.1:8797"
export PATH="$REPO/target/release:$PATH"
cd "$WORK" || exit 1
export PS1='\[\e[32m\]$\[\e[0m\] '
sleep 0.5
clear
