#!/usr/bin/env bash
# Gece yük sınavı: 1.000 eşzamanlı WebSocket bağlantısı + dakikada 10.000 emir.
# CANLIYA DOKUNMAZ: ayrı, geçici bir client-gateway --demo örneği (kendi
# içindeki simülasyon LP'si, /tmp journal'ı, 127.0.0.1:18088) kurulur, LMAX'e
# tek emir gitmez, canlı journal'a yazılmaz. CPU sınırlı (sunucu 2, yük 1 çekirdek).
# Simülatöre giden emirlerde LMAX freni (80/sn) kapalı: motorun kendi kapasitesi ölçülür.
# Sonuç: /var/lib/fxvps-yedek/yuk-sinavi/<zaman>.json + latest.json,
# durum: yuk.durum (OK/HATA) → konsol uyarı motoru ve dashboard.
set -euo pipefail
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
DIR=${FXVPS_DIR:-/opt/fxvps.ai/deploy/lmax-demo}
OUT=${YUK_DIR:-/var/lib/fxvps-yedek}
CLIENTS=${CLIENTS:-1000}
PER_MIN=${PER_MIN:-10000}
DURATION=${DURATION:-120}
PORT=${PORT:-18088}
cd "$DIR"
set -a; . ./.env; set +a
PREFIX=${FIXVPS_TAG_PREFIX:-ghcr.io/fertheoz/fxvps}
TAG=${FIXVPS_TAG:-latest}
STAMP=$(date -u +%Y%m%d-%H%M)
mkdir -p "$OUT/yuk-sinavi"
GW=fxvps-yuk-gw

fail() { echo "HATA $(date -u +%FT%TZ) $*" > "$OUT/yuk.durum"; echo "HATA: $*" >&2; docker rm -f $GW >/dev/null 2>&1 || true; exit 2; }
docker pull -q "$PREFIX-client-gateway:$TAG" >/dev/null
docker pull -q "$PREFIX-loadgen:$TAG" >/dev/null
docker rm -f $GW >/dev/null 2>&1 || true
docker run -d --name $GW --network host --cpus 2 --memory 2g \
  -e FXVPS_MAX_CONNECTIONS=0 -e FXVPS_MAX_CONNECTIONS_PER_IP=0 -e FXVPS_MAX_CONNECTIONS_PER_SUBJECT=0 \
  -e FXVPS_ORDERS_PER_SECOND=500 -e FXVPS_ORDER_BURST=1000 -e RUST_LOG=warn \
  -e FIX_MAX_ORDERS_PER_SEC=0 \
  "$PREFIX-client-gateway:$TAG" --demo --listen "127.0.0.1:$PORT" --data-dir /tmp/yuk >/dev/null
for i in $(seq 1 60); do curl -sf --max-time 1 "http://127.0.0.1:$PORT/healthz" >/dev/null && break; sleep 1; done
curl -sf --max-time 1 "http://127.0.0.1:$PORT/healthz" >/dev/null || fail "sınav örneği açılmadı"

RATE=$(python3 -c "print($PER_MIN / 60 / $CLIENTS)")
RAW="$OUT/yuk-sinavi/$STAMP.raw.json"
docker run --rm --network host --cpus 1 --memory 1g "$PREFIX-loadgen:$TAG" \
  --url "ws://127.0.0.1:$PORT/ws" --clients "$CLIENTS" --duration "$DURATION" \
  --orders-per-sec "$RATE" --ramp-ms 3 --accounts DEMO-1,DEMO-2,DEMO-3 --json > "$RAW" || fail "loadgen çöktü"
docker rm -f $GW >/dev/null 2>&1 || true

python3 - "$RAW" "$OUT" "$STAMP" "$CLIENTS" "$PER_MIN" "$DURATION" <<'EOF'
import json, sys, datetime
raw, out, stamp, clients, per_min, duration = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4]), int(sys.argv[5]), float(sys.argv[6])
r = json.load(open(raw))
ramp = clients * 0.003
active_min = max(0.01, (r.get("duration_s", duration) - ramp) / 60)
sent = r.get("orders_sent", 0)
acks, rejects = r.get("acks", 0), r.get("rejects", 0)
ack = r.get("ack_latency", {})
s = {
    "at": datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0, tzinfo=None).isoformat() + "Z",
    "clients": clients,
    "connected": r.get("connected", 0),
    "ordersSent": sent,
    "ordersPerMin": sent / active_min,
    "targetPerMin": per_min,
    "acks": acks,
    "rejects": rejects,
    "rejectPct": (100.0 * rejects / sent) if sent else 0.0,
    "ackP50Ms": ack.get("p50_ms", 0), "ackP99Ms": ack.get("p99_ms", 0), "ackMaxMs": ack.get("max_ms", 0),
    "quotesPerS": r.get("quotes_per_s", 0),
    "errors": r.get("errors", {}),
}
checks = {
    "bağlantı ≥ %99": s["connected"] >= 0.99 * clients,
    "emir hızı ≥ %95 hedef": s["ordersPerMin"] >= 0.95 * per_min,
    "yanıt ≥ %99": sent > 0 and (acks + rejects) >= 0.99 * sent,
    "ret < %1": s["rejectPct"] < 1.0,
    "yanıt p99 < 250 ms": s["ackP99Ms"] < 250,
}
s["ok"] = all(checks.values())
s["checks"] = checks
doc = {"summary": s, "raw": r}
json.dump(doc, open(f"{out}/yuk-sinavi/{stamp}.json", "w"), ensure_ascii=False, indent=1)
json.dump(doc, open(f"{out}/yuk-sinavi/latest.json", "w"), ensure_ascii=False, indent=1)
line = (f"{s['connected']}/{clients} bağlantı · {s['ordersPerMin']:.0f}/{per_min} emir/dk · "
        f"yanıt p50 {s['ackP50Ms']:.0f} / p99 {s['ackP99Ms']:.0f} ms · ret %{s['rejectPct']:.2f} · fiyat {s['quotesPerS']:.0f}/sn")
failed = [k for k, v in checks.items() if not v]
open(f"{out}/yuk.durum", "w").write(("OK " if s["ok"] else "HATA ") + s["at"] + " " + line + ("" if s["ok"] else " · düşen: " + ", ".join(failed)) + "\n")
print(("OK " if s["ok"] else "HATA ") + line)
EOF
rm -f "$RAW"
find "$OUT/yuk-sinavi" -name '2*.json' -mtime +30 -delete
