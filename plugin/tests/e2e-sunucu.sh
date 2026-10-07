#!/bin/bash
# MT5 bridge end-to-end: E1 (orders) + E2 (connection cut with orders in flight)
set -u
IMG=ghcr.io/fertheoz/fxvps-client-gateway:${TAG:-latest}
KEY=e2e-$(date +%s)
SHA=$(printf %s "$KEY" | sha256sum | cut -d' ' -f1)
echo "[{\"id\":\"kurum-e2e\",\"key_sha256\":\"$SHA\",\"account\":\"DEMO-2\",\"orders_per_sec\":500}]" > /root/brkurum.json
start_gw() {  # fresh demo engine
  docker rm -f br-gw >/dev/null 2>&1
  docker run -d --name br-gw --network host -v /root/brkurum.json:/k.json:ro \
    -e FXVPS_BRIDGE_FILE=/k.json -e FXVPS_DEMO_BALANCE=1000000000 -e FXVPS_MAX_CONNECTIONS_PER_IP=0 -e RUST_LOG=warn,client_gateway::bridge=info \
    $IMG --demo --listen 127.0.0.1:18288 --data-dir /tmp/d >/dev/null
  for i in $(seq 1 60); do curl -sf 127.0.0.1:18288/healthz >/dev/null && return 0; sleep 1; done
  echo "gateway did not start"; docker logs br-gw | tail -5; return 1
}
docker pull -q $IMG >/dev/null
bash /root/derle-sunucu.sh 'core/*.cpp net/ws_posix.cpp tests/e2e.cpp -lpthread -o /p/e2e' | grep -E 'error' && exit 1
RUN="docker run --rm --network host -v /root/pl/plugin:/p gcc:14 /p/e2e ws://127.0.0.1:18288/bridge kurum-e2e $KEY"
echo "=== E1"; start_gw || exit 1; $RUN ${N:-1000} 0; E1=$?
echo "=== E2 (cut at 300)"; start_gw || exit 1; $RUN 600 300; E2=$?
docker logs br-gw 2>&1 | grep -iE "bridge" | tail -3
docker rm -f br-gw >/dev/null
echo "E1=$E1 E2=$E2"
