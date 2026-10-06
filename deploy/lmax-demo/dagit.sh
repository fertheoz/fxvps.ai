#!/usr/bin/env bash
# fxvps.ai demo — dağıtım kapısı (Etap 10). Kullanım: ./dagit.sh [servis...]
# (varsayılan: trading console terminal)
# 1) imajları çek; 2) YENİ motor imajı bugünkü journal kopyasını replay edip
#    değişmezleri geçmeli (şema/replay regresyonu kapıda kalır);
# 3) çalışan imajları geri-donus-<zaman> diye etiketle; 4) up -d;
# 5) 120 sn içinde en az bir FIX oturumu logged_on olmazsa geri al (çıkış 3).
set -euo pipefail
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

DIR=${FXVPS_DIR:-/opt/fxvps.ai/deploy/lmax-demo}
cd "$DIR"
set -a; . ./.env; set +a
SVCS=${*:-trading console terminal}
PREFIX=${FIXVPS_TAG_PREFIX:-ghcr.io/fertheoz/fxvps}
TAG=${FIXVPS_TAG:-latest}
STAMP=$(date -u +%m%d-%H%M)

image_of() { # compose servisi -> imaj adı (etiketsiz)
  case "$1" in
    trading) echo "$PREFIX-client-gateway" ;;
    console) echo "$PREFIX-backoffice-console" ;;
    terminal) echo "$PREFIX-terminal-trade" ;;
    identity) echo "$PREFIX-identity" ;;
    lp-simulator) echo "$PREFIX-lp-simulator" ;;
    *) return 1 ;;
  esac
}

docker compose --profile tunnel pull -q $SVCS
docker pull -q "$PREFIX-core-engine:$TAG" >/dev/null

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
for f in snapshot.json journal.jsonl; do [ -e "core-data/$f" ] && cp "core-data/$f" "$TMP/"; done
if [ -e "$TMP/journal.jsonl" ]; then
  R=$(docker run --rm --network none -v "$TMP:/data:ro" "$PREFIX-core-engine:$TAG" verify --data-dir /data 2>&1) || { echo "KAPI: yeni imaj journal'ı replay edemedi: $R" >&2; exit 2; }
  echo "$R" | grep -q '"ok":true' || { echo "KAPI: değişmezler bozuk: $R" >&2; exit 2; }
  echo "kapı OK: $R"
fi

for s in $SVCS; do
  id=$(docker compose ps -q "$s" 2>/dev/null | head -1 | xargs -r docker inspect --format '{{.Image}}' 2>/dev/null || true)
  if [ -n "$id" ] && img=$(image_of "$s"); then docker tag "$id" "$img:geri-donus-$STAMP"; fi
done

docker compose --profile tunnel up -d $SVCS
for i in $(seq 1 24); do
  sleep 5
  if curl -sf --max-time 3 http://127.0.0.1:9890/status | grep -q '"logged_on":true'; then
    echo "SAĞLIK OK ($((i * 5)) sn): $(curl -s --max-time 3 http://127.0.0.1:9890/status | grep -o '"logged_on":true' | wc -l) oturum açık"
    echo "$(date -u +%FT%TZ) dagit $SVCS OK (geri dönüş: geri-donus-$STAMP)" >> dagit.log
    exit 0
  fi
done
echo "SAĞLIK BAŞARISIZ: geri alınıyor (geri-donus-$STAMP)" >&2
FIXVPS_TAG="geri-donus-$STAMP" docker compose --profile tunnel up -d $SVCS
echo "$(date -u +%FT%TZ) dagit $SVCS GERİ ALINDI → geri-donus-$STAMP" >> dagit.log
exit 3
