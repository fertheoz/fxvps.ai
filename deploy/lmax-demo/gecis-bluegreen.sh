#!/usr/bin/env bash
# Tek seferlik geçiş: tek "trading" sürecinden NATS + ayrı LP gateway'leri +
# mavi/yeşil trading'e. Bir kez ~30-60 sn kesinti (LMAX tek CompID ile iki
# oturuma izin vermez: eski trading'in oturumu kapanmadan yenisi açılamaz).
# Sonraki her dağıtım ./dagit.sh ile kesintisiz.
set -euo pipefail
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
DIR=${FXVPS_DIR:-/opt/fxvps.ai/deploy/lmax-demo}
cd "$DIR"
STAMP=$(date -u +%m%d-%H%M)

mkdir -p nats-data
# override: eski "trading" anahtarı yeni iki renge taşınır (MFA/log ayarları korunur)
if [ -f docker-compose.override.yml ] && grep -q '^  trading:' docker-compose.override.yml; then
  cp docker-compose.override.yml "docker-compose.override.yml.yedek-$STAMP"
  python3 - <<'EOF'
import re
p = 'docker-compose.override.yml'
s = open(p).read()
m = re.search(r'^  trading:\n((?:    .*\n|\n)*)', s, re.M)
block = m.group(1)
s = s[:m.start()] + '  trading-blue:\n' + block + '  trading-green:\n' + block + s[m.end():]
open(p, 'w').write(s)
print('override: trading -> trading-blue + trading-green')
EOF
fi
docker compose config -q
docker compose --profile tunnel pull -q nats lp-gateway-lmax lp-gateway-sim trading-blue console terminal

docker image inspect "$(docker compose ps -q trading 2>/dev/null | head -1 | xargs -r docker inspect --format '{{.Image}}')" >/dev/null 2>&1 \
  && docker tag "$(docker compose ps -q trading | head -1 | xargs docker inspect --format '{{.Image}}')" "ghcr.io/fertheoz/fxvps-client-gateway:geri-donus-$STAMP" || true

T0=$(date +%s)
echo "== eski trading durduruluyor (son snapshot)"
docker stop -t 30 lmax-demo-trading-1 2>/dev/null || true
docker rm lmax-demo-trading-1 2>/dev/null || true
echo "== nats + LP gateway'leri"
docker compose up -d nats
sleep 2
docker compose up -d lp-gateway-lmax lp-gateway-sim
for i in $(seq 1 60); do
  curl -sf --max-time 2 http://127.0.0.1:9890/status | grep -q '"logged_on":true' && break
  sleep 1
done
curl -s --max-time 2 http://127.0.0.1:9890/status | python3 -c 'import json,sys; print("LMAX oturumları:", [(r["kind"], r["logged_on"]) for r in json.load(sys.stdin)])'
echo "== trading-blue (aktif) + trading-green (yedek)"
docker compose up -d --no-deps trading-blue
for i in $(seq 1 120); do curl -sf --max-time 1 http://127.0.0.1:8088/healthz >/dev/null && break; sleep 1; done
echo "trading-blue açık ($(( $(date +%s) - T0 )) sn kesinti)"
docker compose up -d --no-deps trading-green
docker compose up -d --no-deps console terminal
for i in $(seq 1 120); do docker compose logs --no-color trading-green 2>/dev/null | grep -q "standby warm" && break; sleep 1; done
docker compose logs --no-color trading-green | grep "standby" | tail -2
echo "$(date -u +%FT%TZ) GEÇİŞ mavi/yeşil tamam ($(( $(date +%s) - T0 )) sn)" | tee -a dagit.log
