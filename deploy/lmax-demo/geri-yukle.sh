#!/usr/bin/env bash
# fxvps.ai demo — yedekten geri yükleme (Etap 10). Kullanım:
#   ./geri-yukle.sh /var/lib/fxvps-yedek/fxvps-YYYYMMDD-HHMMSS.tgz.enc [--uygula]
# --uygula olmadan yalnız çözer, replay eder ve raporlar (kuru koşum).
# --uygula: servisleri durdurur, mevcut veriyi .eski-<zaman> diye kenara alır,
# yedeği yerine koyar, identity DB'yi dump'tan kurar, servisleri başlatır.
set -euo pipefail
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

ENC=${1:?yedek dosyası}
UYGULA=${2:-}
DIR=${FXVPS_DIR:-/opt/fxvps.ai/deploy/lmax-demo}
KEY=${YEDEK_KEY:-/etc/fxvps/yedek.key}
cd "$DIR"
set -a; . ./.env; set +a
IMG="${FIXVPS_TAG_PREFIX:-ghcr.io/fertheoz/fxvps}-core-engine:${FIXVPS_TAG:-latest}"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

sha256sum -c "${ENC%.tgz.enc}.sha256" --quiet 2>/dev/null && echo "sha256 OK" || echo "UYARI: sha256 dosyası yok/uyuşmuyor"
openssl enc -d -aes-256-cbc -pbkdf2 -iter 200000 -in "$ENC" -pass "file:$KEY" | tar -C "$TMP" -xzf -
echo "çözüldü: $(du -sh "$TMP/yedek" | cut -f1)"
REPORT=$(docker run --rm --network none -v "$TMP/yedek/core-data:/data:ro" "$IMG" verify --data-dir /data)
echo "replay: $REPORT"
echo "$REPORT" | grep -q '"ok":true' || { echo "yedek motor değişmezlerini geçemedi; uygulanmadı" >&2; exit 2; }
[ "$UYGULA" = "--uygula" ] || { echo "kuru koşum bitti (--uygula ile yerine koy)"; exit 0; }

STAMP=$(date -u +%m%d-%H%M)
docker compose --profile tunnel stop trading console terminal identity
mv core-data "core-data.eski-$STAMP"
mkdir core-data && cp -r "$TMP/yedek/core-data/." core-data/ && chown -R 65532:65532 core-data
[ -e "$TMP/yedek/fix-store/lp.json" ] && { cp "$TMP/yedek/fix-store/lp.json" fix-store/lp.json; chmod 600 fix-store/lp.json; chown 65532:65532 fix-store/lp.json; }
docker compose --profile tunnel start identity
sleep 5
docker compose exec -T postgres psql -U fxvps -d postgres -c "DROP DATABASE IF EXISTS identity_restore; CREATE DATABASE identity_restore;" >/dev/null
docker compose exec -T postgres psql -U fxvps -d identity_restore < "$TMP/yedek/identity.sql" >/dev/null
docker compose --profile tunnel stop identity
docker compose exec -T postgres psql -U fxvps -d postgres -c "ALTER DATABASE identity RENAME TO identity_eski_$STAMP; ALTER DATABASE identity_restore RENAME TO identity;" >/dev/null
docker compose --profile tunnel up -d identity trading console terminal
echo "$(date -u +%FT%TZ) GERİ YÜKLENDİ $ENC (eski veri: core-data.eski-$STAMP, identity_eski_$STAMP)" | tee -a dagit.log
