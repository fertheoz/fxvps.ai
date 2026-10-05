# LP konsolu — LMAX demo (CT 970)

Geçici kurulum: **fix-gateway** + **core-engine** + **back office konsolu**. LP FIX 4.4 bilgileri dosyaya yazılmaz, konsoldaki LP sayfasından girilir. Kaydedince gateway oturumları yeniden başlatır, durum aynı sayfada görünür.
Kurulum mt5forexvps altyapısında, ödünç bir LXC'de yapılır; adımlar [`BRIFING-mt5forexvps.md`](BRIFING-mt5forexvps.md) dosyasında.

| Dosya | İçerik |
|---|---|
| `docker-compose.yml` | Üç servis, hepsi yalnız `127.0.0.1`'de (CT içinde host ağı) |
| `console-nginx.conf` | Konsol: statik sayfalar + `/v1`, `/auth` → core-engine (aynı köken) |
| `.env.example` | `FIX_ADMIN_TOKEN` (gateway ↔ core-engine ortak sırrı), imaj etiketi |

| Port (CT loopback) | Servis |
|---|---|
| 8080 | Konsol (tarayıcı, SSH tüneliyle) |
| 8090 | core-engine admin API (dev-token girişi) |
| 9890 | fix-gateway `/status`, `/config` (token korumalı) |

Güvenlik:
- Hiçbir port CT dışına açılmaz. Konsola erişim yalnız SSH tüneliyle olur.
- Dev-token girişi, core-engine loopback'te olduğu için açık. Tünele erişen herkes admin olur; tüneli yalnız kurucu açar.
- LP parolaları konsoldan yalnız yazılır, geri okunmaz. CT'de `fix-store/lp.json` dosyasında (0600) durur. Denetim kaydında yalnız "password changed" görünür.
