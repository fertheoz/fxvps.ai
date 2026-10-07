#!/usr/bin/env bash
# MT5 köprüsüne kurum ekler: rastgele anahtar üretir (YALNIZ BİR KEZ gösterilir),
# SHA-256 özetini core-data/bridge/kurumlar.json'a yazar (köprü 2 sn'de okur).
# Tercih edilen yol: konsol → Kurumlar.
#   ./kurum-ekle.sh <kurum-id> <kurum-hesabı> [emir/sn=100] [ip,ip]
# Kurum hesabı konsolda önceden açılmış, NETTING grubunda olmalı.
set -euo pipefail
cd "${FXVPS_DIR:-/opt/fxvps.ai/deploy/lmax-demo}"
ID=${1:?kurum-id}; ACC=${2:?kurum hesabı}; RATE=${3:-100}; IPS=${4:-}
B=core-data/bridge; mkdir -p $B; [ -f $B/kurumlar.json ] || echo '[]' > $B/kurumlar.json
KEY=$(head -c 32 /dev/urandom | base64 | tr -d '/+=' | cut -c1-40)
SHA=$(printf %s "$KEY" | sha256sum | cut -d' ' -f1)
cp $B/kurumlar.json "$B/kurumlar.json.yedek-$(date -u +%m%d-%H%M)"
python3 - "$ID" "$ACC" "$RATE" "$IPS" "$SHA" <<'PY'
import json, sys
i, acc, rate, ips, sha = sys.argv[1:]
p = "core-data/bridge/kurumlar.json"
l = [k for k in json.load(open(p)) if k["id"] != i]
l.append({"id": i, "key_sha256": sha, "account": acc, "orders_per_sec": int(rate),
          "ips": [x for x in ips.split(",") if x]})
json.dump(l, open(p, "w"), indent=1)
PY
chown -R 65532:65532 $B
echo "kurum $ID -> hesap $ACC eklendi. ANAHTAR (bir daha gösterilmez): $KEY"
echo "plugin yapılandırması: institution=$ID  key=<yukarıdaki>  endpoints=wss://trade.fxvps.ai/bridge"
# köprü dosyayı 2 sn içinde kendisi yeniden okur; yeniden başlatma gerekmez
