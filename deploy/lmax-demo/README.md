# fxvps.ai demo platformu — CT 970 (LMAX demo)

Konsol (`console.fxvps.ai`, Cloudflare Access + kimlik girişi), müşteri terminali
(`trade.fxvps.ai`), kimlik servisi (`id.fxvps.ai`) ve tek `trading` süreci
(client-gateway + core-engine + fix-gateway → LMAX demo FIX). Hepsi CT içinde
yalnız 127.0.0.1'de; yayın Cloudflare Tunnel ile, içeri port açılmaz.

Kurulum, doğrulama, ilk admin, LP ayarı ve müşteri hesabı:
[`BRIFING-mt5forexvps.md`](BRIFING-mt5forexvps.md).

| Dosya | İçerik |
|---|---|
| `docker-compose.yml` | postgres, identity, trading, console, terminal, cloudflared (profile `tunnel`) |
| `console-nginx.conf` | konsol + `/v1`, `/auth` → admin API (127.0.0.1:8090) |
| `terminal-nginx.conf` | terminal + `/ws` → client-gateway (127.0.0.1:8088) |
| `.env.example` | `FIX_ADMIN_TOKEN`, `PG_PASSWORD`, `CLOUDFLARE_TUNNEL_TOKEN` |
