#!/usr/bin/env bash
# Gece köprü sınavı (CT 970): depodaki plugin'i derler, E1+E2'yi koşar,
# sonucu /var/lib/fxvps-yedek/kopru.durum'a yazar (OK/HATA).
set -u
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
REPO=${FXVPS_REPO:-/opt/fxvps.ai}
OUT=/var/lib/fxvps-yedek/kopru.durum
tar czf /root/plugin.tgz -C "$REPO" plugin
cp "$REPO/plugin/tests/derle-sunucu.sh" /root/derle-sunucu.sh
LOG=$(bash "$REPO/plugin/tests/e2e-sunucu.sh" 2>&1)
SUM=$(echo "$LOG" | grep -E "^orders=" | tr '\n' ' ')
if echo "$LOG" | grep -q "E1=0 E2=0"; then
  echo "OK $(date -u +%FT%TZ) $SUM" > "$OUT"
else
  echo "HATA $(date -u +%FT%TZ) $SUM $(echo "$LOG" | grep -E 'FAIL|error' | head -3 | tr '\n' ' ')" > "$OUT"
fi
cat "$OUT"
