# Checkpoint — 6 Ekim 2026

Yeni bir oturum (yerel ya da bulut) buradan başlar. Önceki özet: [`CHECKPOINT-2026-10-05.md`](CHECKPOINT-2026-10-05.md).
Sabit nokta: `main` @ `d293726` (#58), CT 970'te canlı. Son güncelleme: 6 Ekim 01:10 (TSİ).

## 5–6 Ekim gecesi `main`'e birleşenler

| PR | İçerik |
|---|---|
| #50 | Terminal: üst çubukta "Simulator" yerine gerçek gateway; 60 sn'lik (admin) jeton artık 5 sn'de bir değil ~45 sn'de bir yenilenir; oturum bitip yeniden girilince gateway'e tekrar bağlanır |
| #51 | **Grafik geçmişi:** mumlar `<data-dir>/candles.txt`'ye yazılır (5 dk'da bir), açılışta yüklenir; `candles-seed.txt` ile geri doldurma; `deploy/lmax-demo/seed-candles.py` |
| #52 | **Kimlik:** uygulama başına ayrı yenileme çerezi (`fxvps_rt_<host>`); konsol ve terminal artık birbirinin oturumunu düşürmez; konsol oturumu kalmayınca yenilemeyi durdurur |
| #53 | Gateway abonelikte sembol başına **son fiyatı** hemen gönderir (Market Watch dakikalarca boş kalmaz) |
| #54, #56 | **Mobil arayüz** (768 px altı): Piyasa · Grafik · Portföy · Hesap, alttan açılan emir fişi (`OrderTicket variant="sheet"`) |
| #55 | Terminal: oturum yenileme sekmeler arasında Web Lock ile sıraya girer |
| #57 | **Konsol:** LP işlemleri dökümü, gelir raporu (`/v1/reports/lp-executions`, `/v1/reports/revenue`), panelde gerçek A-book geliri ve LP oturum sayısı |
| #58 | nginx: HTML `Cache-Control: no-cache` (dağıtımdan sonra eski sürüm önbellekten açılmaz) |

## Durum (6 Ekim 01:10 TSİ)

- **İlk gerçek uçtan uca işlem tamam.** 100001 hesabı, EURUSD 0.01 lot: LMAX'te 1.12203 alış → 1.12230 satış; müşteriye 1.12208 → 1.12225. Defter: LMAX sonucu +0,27, müşteri net +0,09, broker geliri +0,18 USD (markup 0,10 + komisyon 0,08).
- LP: MD + TRADING `logged_on`. Önceki checkpoint'teki 0, 0b, 2 (kısmen), 3, 4 kapandı.
- Motor 6 Ekim'de işlem günlüğünden baştan oynatıldı (yeni alanlar eski işlemlere de dolsun diye); bakiyeler birebir tuttu. Yedekler: `core-data/snapshot.json.yedek-1006`, `journal.jsonl.yedek-1006`. Geri dönüş imajları: `geri-donus-1005`, `geri-donus-1006` etiketleri.
- Grafik geçmişi: 83 parite × 7 zaman dilimi, 346.343 mum yüklendi. Madenler (XAU/XAG/XPD/XPT) ve CNHSEK kaynakta yok, canlı akıştan dolar.
- **Kurucu doğrulaması bekleyenler** (sunucu tarafı ölçüldü, tarayıcıdan görülmedi): oturumun artık düşmediği; grafiklerin dolu açıldığı; konsolda yeni panel kartı ve iki rapor sekmesi. Mobil arayüzün telefonda açıldığını kurucu teyit etti.

## Açık işler (sırayla)

0c. EURDKK / EURILS fiyat seyrek — LMAX ID'leri (100485 / 100989) doğrulanmadı.

1. **SMTP (Resend):** `fxvps.ai` alanı + DNS + API anahtarı; CT `.env`'e `IDENTITY_SMTP_URL`, `IDENTITY_MAIL_FROM`; `docker compose up -d identity`.
2. **Saygın Balıkel:** konsol → Hesap aç (`saygin.balikel@gmail.com`, `demo-retail`) → 5.000 USD → kayıt → `link-account` (SMTP yoksa önce `verify-email`).
3. **Grafik geçmişi kaynağı:** `seed-candles.py` Yahoo Finance'in anahtarsız ucunu kullanıyor; yalnız demo için. Fiyatlar LMAX'ten ~1 pip farklı olabilir ve kaynak yeniden dağıtım için lisanslı değil → canlıdan önce lisanslı kaynak (LMAX REST geçmiş verisi ayrı API anahtarı ve demo için LMAX desteğinden izin istiyor; doğrulanmadı).
4. **Mobil:** gerçek telefonda ve açık temada gözden geçirme; mobilde hesap güvenliği (2FA kurulumu) ekranı yok; PWA service worker yok. Native uygulama (`apps/mobile`, Expo) aynı tasarımı almadı.
5. **Konsol paneli:** gelir raporu gerçekleşen (kapanmış) işlemleri gösterir; açık pozisyonların birikmiş markup'ı ve günlük/parite bazında gelir grafiği yok. `pnlSeries` hâlâ boş.
6. Cloudflare `*.fxvps.ai` joker A kaydı — kurucu kararı (önceki checkpoint'ten).

## Bilinen sınırlar / notlar

- Yerelde (Windows) Rust derlenmez (OpenSSL yok): `cargo fmt` çalışır, clippy ve testler yalnız CI'da. Terminal ve konsol yerelde `npx pnpm@9.15.0 typecheck|lint|test` ile doğrulanır; konsolun iki `token-storage` testi Node 26'da yerelde düşer (CI'da geçer).
- CI: her PR ~19 iş açar; 4–5 PR aynı anda "job was not acquired by Runner" ile düşer (kod hatası değil) → `gh run rerun <id> --failed`.
- `trading` yeniden başlayınca son-fiyat önbelleği boşalır; her parite ilk tikini bekler. New York kapanışında (21:00 UTC civarı) LMAX birkaç dakika durur, tikler seyrekleşir.
- Önceki checkpoint'teki kurallar geçerli: sırlar (LMAX parolası, tünel jetonu, SMTP anahtarı) sohbete ve depoya yazılmaz. CT 970 işlemleri bu oturumda kurucu onayıyla doğrudan yapıldı (`ssh mt5` → `pct exec 970`, `/opt/fxvps.ai/deploy/lmax-demo`, `docker compose --profile tunnel`).
- Uzun vadeli işler sürüyor: NATS `CoreApi`, mobil EAS, masaüstü imzalı sürüm, performans, LMAX `[DOĞRULA]` maddeleri (`docs/02`).

## Yerelde başlamak

```bash
cd D:\fxvps.ai && git pull
claude        # ilk mesaj: "docs/CHECKPOINT-2026-10-06.md'yi oku, açık işlerden devam et"
```
