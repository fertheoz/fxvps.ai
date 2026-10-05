# Checkpoint — 4 Ekim 2026 (son durum)

Bu belge, bulut oturumu kapanmadan önce projenin **tek kaynaklı özetidir**. Yeni bir oturum buradan başlamalıdır.
Sabit nokta: `checkpoint/2026-10-04` dalı = `main` @ `cc766cf`.

## Proje

**fxvps.ai** — LP'lere (ör. LMAX) FIX 4.4 ile bağlanan, müşterilere alt hesap + MT5/cTrader sınıfı terminal veren
çok kiracılı trading platformu. Depo: `github.com/fertheoz/fxvps.ai`.

## `main`'e birleşenler (16 PR, hepsi CI yeşil)

| PR | İçerik |
|---|---|
| #1 | M0 araştırma: rakip analizi (MT5/cTrader), FIX/LMAX, teknoloji mimarisi → `docs/01-03` |
| #2 | M1: FIX 4.4 codec, oturum katmanı, LMAX benzeri LP simülatörü, fix-gateway |
| #3 | M3: web terminali (React 19 + Vite, grafik, emir bileti, DoM, EN/TR) |
| #4 | M4: back office (Next.js 16, RBAC) |
| #5 | client-gateway: protobuf/JSON WebSocket protokolü, JWT, demo modu |
| #6 | Altyapı: Dockerfile'lar, compose, Helm, GHCR + Pages iş akışları → `docs/04` |
| #7 | M2: money, çift taraflı defter, risk (ESMA), OMS, core-engine |
| #8 | M5: Tauri 2 masaüstü, Expo mobil, `packages/trading-core` |
| #9 | Terminal ↔ gateway gerçek protokol + canlı e2e |
| #10 | Çekirdek entegrasyonu: gateway → core-engine → FIX LP; gerçek bakiye/K-Z |
| #11 | Protokol v1.2: stop/stop-limit, SL/TP/trailing, hedging, kısmi kapanış, OCO, GTD, geçmiş |
| #12 | Kimlik servisi: kayıt/giriş, 2FA, passkey, dönen oturumlar, JWKS |
| #13 | Back office ↔ core-engine admin API (RBAC, 4-göz, idempotency, denetim, SSE) |
| #14 | Yük testi aracı (`tools/loadgen`), güvenlik incelemesi → `docs/05`, `docs/06` |
| #15 | Güvenlik düzeltmeleri G1–G13, G16 → `docs/07` |
| #22 | Masaüstü köprüsü (grafik pencere ayırma, tepsi, bildirim) + mobil canlı WS modu |

## Kod haritası

- `crates/`: domain, money, fix-codec, fix-session, client-proto, ledger, risk, oms
- `services/`: lp-simulator, fix-gateway, core-engine, client-gateway, identity
- `apps/`: terminal (web), backoffice, desktop (Tauri), mobile (Expo)
- `packages/trading-core`, `tools/loadgen`, `infra/docker`, `deploy/{compose,helm}`, `.github/workflows`

## Hızlı başlangıç

```bash
cargo run -p client-gateway -- --demo      # simülatör + FIX + çekirdek; URL ve demo token stdout'ta
cd apps/terminal && pnpm install --ignore-workspace && pnpm dev
# tarayıcı: http://localhost:5173 → üst bar "Gateway" → FXVPS_WS_URL + FXVPS_DEMO_TOKEN
```
Ayrıntı: kök `README.md`, `docs/04-operasyon.md`.

## Açık işler (sonraki oturum)

1. **GitHub Pages (elle):** Settings → Pages → Source: **GitHub Actions**. Sonraki `main` push'unda terminal
   `https://fertheoz.github.io/fxvps.ai/`, back office `/admin/` altında yayınlanır. (Depo private ise ücretli plan gerekir.)
2. **Depo görünürlüğü:** şu an public; private için Settings → Danger Zone.
3. **Güvenlik:** G14, G15, G17 ertelendi; G8, G10, G13 kısmi (`docs/07`).
4. **Ürün eksikleri:**
   - ~~Admin API'de LP oturumları ve işlem geçmişi boş~~ (tamam: #25, #26; ayrı süreçte `FIX_STATUS_ADDR` + `CORE_LP_STATUS_URL`).
   - NATS tabanlı `CoreApi` yok.
   - Terminal henüz `packages/trading-core`'u kullanmıyor (kopya kod).
   - Mobil EAS native derleme ve push bildirimi yok.
   - Masaüstü imzalı sürüm + updater anahtarları yok.
5. **LMAX:** gerçek FIX spesifikasyonu onboarding'de alınmalı; `docs/02`'deki `[DOĞRULA]` maddeleri teyit edilmeli.
6. **Performans:** sandbox ölçümünde dolum gecikmesi iki tepeli (~1,7 ms ve ~45 ms); kök neden araştırılmadı (`docs/05`).

## Kalan dallar

Tüm iş `main`'de; `feat/*`, `fix/*`, `docs/*` dalları birleşmiş durumda, silinebilir. `checkpoint/2026-10-04` korunmalı.
