#!/usr/bin/env bash
# fxvps.ai demo (CT 970) — şifreli yedek + geri yükleme provası (bridge Etap 10).
#
# Ne yedekler: motor journal/snapshot/admin defteri/tercihler/mum geçmişi/arşiv,
# LP bağlantı ayarı (lp.json, parola dahil → şifreli), identity Postgres dump'ı,
# imza anahtarları, .env ve compose dosyaları.
# Nereye: $YEDEK_DIR (CT'ye Proxmox host'tan bağlanan dizin; CT diski ölse de kalır),
# AES-256-CBC (openssl, pbkdf2) ile; anahtar $YEDEK_KEY (CT'de 0600, kopyası host'ta).
# Prova: her yedek hemen çözülür ve core-engine imajı journal'ı replay edip
# değişmezleri denetler; başarısızsa çıkış 2 ve yedek.durum "HATA" olur
# (konsol uyarı motoru bu dosyayı izler → sessiz yedek arızası yok).
set -euo pipefail
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

DIR=${FXVPS_DIR:-/opt/fxvps.ai/deploy/lmax-demo}
OUT=${YEDEK_DIR:-/var/lib/fxvps-yedek}
KEY=${YEDEK_KEY:-/etc/fxvps/yedek.key}
KEEP_DAYS=${KEEP_DAYS:-14}
STAMP=$(date -u +%Y%m%d-%H%M%S)
DURUM="$OUT/yedek.durum"

cd "$DIR"
set -a; . ./.env; set +a
IMG="${FIXVPS_TAG_PREFIX:-ghcr.io/fertheoz/fxvps}-core-engine:${FIXVPS_TAG:-latest}"
mkdir -p "$OUT"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

fail() {
  echo "$(date -u +%FT%TZ) HATA $STAMP $*" | tee -a "$OUT/yedek.log" >&2
  echo "HATA $(date -u +%FT%TZ) $*" > "$DURUM"
  exit 2
}

[ -s "$KEY" ] || fail "anahtar yok: $KEY"
mkdir -p "$TMP/yedek/core-data" "$TMP/yedek/fix-store"
# snapshot önce, journal sonra: journal append-only; yarım son satırı recover zaten yok sayar
for f in snapshot.json admin.jsonl prefs.json candles.txt; do
  [ -e "core-data/$f" ] && cp "core-data/$f" "$TMP/yedek/core-data/"
done
cp core-data/journal.jsonl "$TMP/yedek/core-data/"
[ -d core-data/archive ] && cp -r core-data/archive "$TMP/yedek/core-data/"
[ -e fix-store/lp.json ] && cp fix-store/lp.json "$TMP/yedek/fix-store/"
cp -r keys "$TMP/yedek/keys"
for f in .env docker-compose.yml docker-compose.override.yml lp-sim.toml lp-sim-gateway.json console-nginx.conf terminal-nginx.conf; do
  [ -e "$f" ] && cp "$f" "$TMP/yedek/"
done
docker compose exec -T postgres pg_dump -h 127.0.0.1 -p 5433 -U fxvps identity > "$TMP/yedek/identity.sql" || fail "pg_dump"

tar -C "$TMP" -czf "$TMP/yedek.tgz" yedek
ENC="$OUT/fxvps-$STAMP.tgz.enc"
openssl enc -aes-256-cbc -pbkdf2 -iter 200000 -salt -in "$TMP/yedek.tgz" -out "$ENC" -pass "file:$KEY" || fail "şifreleme"
( cd "$OUT" && sha256sum "$(basename "$ENC")" > "fxvps-$STAMP.sha256" )

# geri yükleme provası: çöz, motorla replay et, değişmezleri denetle
mkdir -p "$TMP/prova"
openssl enc -d -aes-256-cbc -pbkdf2 -iter 200000 -in "$ENC" -pass "file:$KEY" | tar -C "$TMP/prova" -xzf - || fail "çözme"
[ -s "$TMP/prova/yedek/identity.sql" ] || fail "identity dump boş"
REPORT=$(docker run --rm --network none -v "$TMP/prova/yedek/core-data:/data:ro" "$IMG" verify --data-dir /data 2>&1) || fail "verify: $REPORT"
echo "$REPORT" | grep -q '"ok":true' || fail "değişmezler: $REPORT"

find "$OUT" -name 'fxvps-*.tgz.enc' -mtime +"$KEEP_DAYS" -delete
find "$OUT" -name 'fxvps-*.sha256' -mtime +"$KEEP_DAYS" -delete
SIZE=$(stat -c %s "$ENC")
echo "$(date -u +%FT%TZ) OK $STAMP ${SIZE}B $REPORT" | tee -a "$OUT/yedek.log"
echo "OK $(date -u +%FT%TZ) $STAMP ${SIZE}B" > "$DURUM"
