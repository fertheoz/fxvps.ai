# Brifing v6 — fxvps.ai → mt5forexvps: klon + CT 970 güncellemesi (5 Ekim 2026)

**Onaylayan:** Kurucu. Sırlar (LMAX parolası, tünel jetonu, SMTP anahtarı) ekrana, log'a yazılmaz.

## 1. PC — depoyu klonla
```
git clone git@github.com:fertheoz/fxvps.ai.git D:\fxvps.ai
```
(SSH anahtarı yoksa `https://github.com/fertheoz/fxvps.ai.git`.) Kurucu bu klasörde yerel Claude Code oturumu açıp `/chrome` bağlayacak.

## 2. CT 970 — yeni imajlar
`main` @ `4def12d` veya sonrası. İmajlar GitHub Actions'ta (`docker` iş akışı, `publish`) derlenir; önce `main` için o koşunun bittiğini kontrol et.
```bash
cd /opt/fxvps.ai && git pull
cd deploy/lmax-demo
docker compose --profile tunnel pull trading console terminal
docker compose --profile tunnel up -d trading console terminal
docker compose --profile tunnel ps
```
`trading` yeniden başlarken LMAX oturumları 5–10 sn düşer; normal.

| PR | Neyi düzeltir | Kap |
|---|---|---|
| #45 | LP **Stop/Play** tuşları; LMAX girişi üst üste 3 kez reddedilirse bağlantı durur (hesap kilitlenmez) | trading, console |
| #46 | 91 enstrüman listesi testi (yalnız CI) | — |
| #47 | Konsol girişinde yalnız e-posta + parola + 2FA ("token yapıştır" ve dev kutusu kalktı) | console |
| #48 | Terminal: giriş jetonunun süresi dolunca kopmuyor, yenilenmiş jetonla yeniden bağlanıyor | terminal |

## 3. Doğrulama
| # | Kontrol | Beklenen |
|---|---|---|
| 1 | `curl -s 127.0.0.1:9890/status` | MD ve TRADING `logged_on` |
| 2 | console.fxvps.ai giriş ekranı | Yalnız e-posta/parola formu |
| 3 | console.fxvps.ai → LP | "Çalışıyor" + **Stop** tuşu; durum tablosunda hata varsa altında `lastError` |
| 4 | trade.fxvps.ai (kurucu) | Sağ üst `Gateway: trade.fxvps.ai` · `Connected` |
| 5 | trade.fxvps.ai'de 20+ dk açık bekle | Journal'da "Session token renewed; reconnecting", sonra yine `Connected` |

Terminal açılışta "Simulator" gösterirse: `https://trade.fxvps.ai/?api=ws&url=wss://trade.fxvps.ai/ws`.

## 4. İlk gerçek emir (kurucu)
trade.fxvps.ai → EUR/USD → lot **0.01** → BUY → Positions'ta görünür → kapat.
Sonra (parolasız satırlar fxvps.ai'ye):
```bash
docker compose logs --since 10m trading | grep -iE "exec|fill|reject|35=8|35=j|35=3" | tail -30
```

## 5. Bilinen
- EURDKK / EURILS fiyatı ara sıra "…": LMAX demo seyrek yayınlıyor olabilir; teste engel değil.
- Saygın bağlanmadıysa: `docker compose exec identity /app/identity link-account saygin.balikel@gmail.com <LOGIN>` → Saygın çıkış/giriş.

## Rapor
Her madde için geçti/kaldı → fxvps.ai. Geri alma: `docker compose stop cloudflared`; hiçbir şey silinmez.
