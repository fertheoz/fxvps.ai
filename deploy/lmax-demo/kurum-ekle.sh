#!/usr/bin/env bash
# MT5 köprüsüne kurum ekler: rastgele anahtar üretir (YALNIZ BİR KEZ gösterilir),
# SHA-256 özetini bridge/kurumlar.json'a yazar, trading'i kesintisiz yeniden yükler.
#   ./kurum-ekle.sh <kurum-id> <kurum-hesabı> [emir/sn=100] [ip,ip]
# Kurum hesabı konsolda önceden açılmış, NETTING grubunda olmalı.
set -euo pipefail
cd "${FXVPS_DIR:-/opt/fxvps.ai/deploy/lmax-demo}"
ID=${1:?kurum-id}; ACC=${2:?kurum hesabı}; RATE=${3:-100}; IPS=${4:-}
mkdir -p bridge; [ -f bridge/kurumlar.json ] || echo '[]' > bridge/kurumlar.json
KEY=$(head -c 32 /dev/urandom | base64 | tr -d '/+=' | cut -c1-40)
SHA=$(printf %s "$KEY" | sha256sum | cut -d' ' -f1)
cp bridge/kurumlar.json "bridge/kurumlar.json.yedek-$(date -u +%m%d-%H%M)"
python3 - "$ID" "$ACC" "$RATE" "$IPS" "$SHA" <<'PY'
import json, sys
i, acc, rate, ips, sha = sys.argv[1:]
p = "bridge/kurumlar.json"
l = [k for k in json.load(open(p)) if k["id"] != i]
l.append({"id": i, "key_sha256": sha, "account": acc, "orders_per_sec": int(rate),
          "ips": [x for x in ips.split(",") if x]})
json.dump(l, open(p, "w"), indent=1)
PY
chmod 755 bridge; chmod 644 bridge/kurumlar.json
echo "kurum $ID -> hesap $ACC eklendi. ANAHTAR (bir daha gösterilmez): $KEY"
echo "plugin yapılandırması: institution=$ID  key=<yukarıdaki>  endpoints=wss://trade.fxvps.ai/bridge"
./dagit.sh trading
