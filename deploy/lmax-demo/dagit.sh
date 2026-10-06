#!/usr/bin/env bash
# fxvps.ai demo — kesintisiz dağıtım kapısı. Kullanım: ./dagit.sh [servis...]
# (varsayılan: trading console terminal)
#
# trading (mavi/yeşil): bekleyen renk yeni imajla açılır ve journal'ı ısıtır,
#   aktif renk durdurulur (SIGTERM → son snapshot → yazıcı kilidi bırakılır),
#   bekleyen ~1 sn içinde devralır; LP oturumları (lp-gateway-*) hiç düşmez,
#   müşteri terminali 300 ms'de yeniden bağlanır. Eski renk yeni imajla
#   bekleyen olur. Devralma 60 sn içinde olmazsa eski imaj geri getirilir.
# Diğer servisler: geri dönüş etiketi + up -d.
# Kapı: YENİ motor imajı bugünkü journal kopyasını replay edip değişmezleri geçmeli.
set -euo pipefail
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

DIR=${FXVPS_DIR:-/opt/fxvps.ai/deploy/lmax-demo}
cd "$DIR"
set -a; . ./.env; set +a
SVCS=${*:-trading console terminal}
PREFIX=${FIXVPS_TAG_PREFIX:-ghcr.io/fertheoz/fxvps}
TAG=${FIXVPS_TAG:-latest}
STAMP=$(date -u +%m%d-%H%M)
log() { echo "$(date -u +%FT%TZ) $*" | tee -a dagit.log; }

image_of() {
  case "$1" in
    trading|trading-blue|trading-green) echo "$PREFIX-client-gateway" ;;
    console) echo "$PREFIX-backoffice-console" ;;
    terminal) echo "$PREFIX-terminal-trade" ;;
    identity) echo "$PREFIX-identity" ;;
    lp-simulator) echo "$PREFIX-lp-simulator" ;;
    lp-gateway-lmax|lp-gateway-sim) echo "$PREFIX-fix-gateway" ;;
    *) return 1 ;;
  esac
}
tag_running() { # servis -> geri-donus-$STAMP etiketi
  local id img
  id=$(docker compose ps -q "$1" 2>/dev/null | head -1 | xargs -r docker inspect --format '{{.Image}}' 2>/dev/null || true)
  if [ -n "$id" ] && img=$(image_of "$1"); then docker tag "$id" "$img:geri-donus-$STAMP"; fi
}
healthy() { curl -sf --max-time 1 "http://127.0.0.1:$1/healthz" >/dev/null 2>&1; }
port_of() { [ "$1" = blue ] && echo 8088 || echo 8089; }

COMPOSE_SVCS=""
for s in $SVCS; do
  if [ "$s" = trading ]; then COMPOSE_SVCS="$COMPOSE_SVCS trading-blue trading-green"; else COMPOSE_SVCS="$COMPOSE_SVCS $s"; fi
done
docker compose --profile tunnel pull -q $COMPOSE_SVCS
docker pull -q "$PREFIX-core-engine:$TAG" >/dev/null

# --- replay kapısı ---------------------------------------------------------
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
for f in snapshot.json journal.jsonl; do [ -e "core-data/$f" ] && cp "core-data/$f" "$TMP/"; done
# the engine image runs as nonroot: the copy must be readable (mktemp -d is 0700)
chmod 755 "$TMP"; chmod 644 "$TMP"/* 2>/dev/null || true
if [ -e "$TMP/journal.jsonl" ]; then
  R=$(docker run --rm --network none -v "$TMP:/data:ro" "$PREFIX-core-engine:$TAG" verify --data-dir /data 2>&1) || { echo "KAPI: yeni imaj journal'ı replay edemedi: $R" >&2; exit 2; }
  echo "$R" | grep -q '"ok":true' || { echo "KAPI: değişmezler bozuk: $R" >&2; exit 2; }
  # a gate that saw nothing is no gate: the replayed state must be the live one
  if [ -s "$TMP/journal.jsonl" ] || [ -s "$TMP/snapshot.json" ]; then
    echo "$R" | grep -q '"seq":0,' && { echo "KAPI: journal okunamadı (seq 0): $R" >&2; exit 2; }
  fi
  echo "kapı OK: $(echo "$R" | cut -c1-160)"
fi

# --- mavi/yeşil trading ------------------------------------------------------
wait_log() { # servis desen süre(sn) başlangıç(RFC3339)
  local i
  for i in $(seq 1 "$3"); do
    docker compose logs --no-color --since "$4" "$1" 2>/dev/null | grep -q "$2" && return 0
    sleep 1
  done
  return 1
}
bluegreen() {
  local A S since t0 t1 gap i
  if healthy 8088; then A=blue; S=green
  elif healthy 8089; then A=green; S=blue
  else
    log "aktif trading yok: iki renk başlatılıyor"
    docker compose up -d trading-blue trading-green
    for i in $(seq 1 120); do healthy 8088 || healthy 8089 && return 0; sleep 1; done
    echo "trading başlamadı" >&2; return 3
  fi
  tag_running "trading-$A"; tag_running "trading-$S"
  since=$(date -u +%FT%TZ)
  docker compose up -d --no-deps --force-recreate "trading-$S"
  wait_log "trading-$S" "standby warm" 300 "$since" || { echo "trading-$S ısınmadı" >&2; return 3; }
  log "trading-$S sıcak bekliyor; trading-$A devrediyor"
  t0=$(date +%s%3N)
  docker compose stop -t 30 "trading-$A" >/dev/null 2>&1 &
  for i in $(seq 1 600); do
    if healthy "$(port_of "$S")"; then
      t1=$(date +%s%3N); gap=$((t1 - t0))
      wait || true
      log "DEVİR OK: trading-$A → trading-$S, ilk yanıt ${gap} ms (eski: geri-donus-$STAMP)"
      since=$(date -u +%FT%TZ)
      docker compose up -d --no-deps --force-recreate "trading-$A"
      wait_log "trading-$A" "standby warm" 300 "$since" && log "trading-$A yeni imajla sıcak yedek" || log "UYARI: trading-$A yedek olarak ısınmadı"
      return 0
    fi
    sleep 0.1
  done
  wait || true
  log "DEVİR BAŞARISIZ: trading-$S 60 sn içinde açılmadı; geri alınıyor"
  docker compose stop -t 5 "trading-$S" || true
  FIXVPS_TAG="geri-donus-$STAMP" docker compose up -d --no-deps "trading-$A"
  return 3
}

for s in $SVCS; do
  if [ "$s" = trading ]; then
    bluegreen || exit 3
  else
    tag_running "$s"
    docker compose --profile tunnel up -d --no-deps "$s"
    log "dagit $s OK"
  fi
done

# --- sağlık: en az bir LP oturumu açık ---------------------------------------
for i in $(seq 1 24); do
  if curl -sf --max-time 3 http://127.0.0.1:9890/status | grep -q '"logged_on":true'; then
    echo "SAĞLIK OK: $(curl -s --max-time 3 http://127.0.0.1:9890/status | grep -o '"logged_on":true' | wc -l) LMAX oturumu açık"
    exit 0
  fi
  sleep 5
done
echo "UYARI: LMAX oturumu açık değil (lp-gateway-lmax'e bakın)" >&2
exit 4
