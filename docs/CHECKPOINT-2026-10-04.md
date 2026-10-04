# Checkpoint — 4 Ekim 2026

Bulut oturumu kredi sınırında durduruldu. Bu belge, `main`'in bu noktadaki durumunu ve yarım kalan işleri kaydeder. Git etiketi: `checkpoint-2026-10-04`.

## `main`'de olanlar (birleşmiş PR'lar, hepsi CI yeşil)

| PR | İçerik |
|---|---|
| #1 | M0 — araştırma: MT5/cTrader rakip analizi, FIX/LMAX entegrasyonu, teknoloji mimarisi |
| #2 | M1 — FIX 4.4 codec, oturum katmanı, LMAX benzeri LP simülatörü, fix-gateway, CI |
| #3 | M3 — web terminali (React 19, Market Watch, grafikler, emir bileti, DoM, EN/TR) |
| #4 | M4 — back office (Next.js 16, RBAC, mock AdminApi) |
| #5 | client-gateway — protobuf/JSON WebSocket protokol v1, JWT, demo modu |
| #6 | Altyapı — Dockerfile'lar, compose geliştirme ortamı, Helm chart, GHCR + Pages iş akışları |
| #7 | M2 — money, çift taraflı defter, risk, OMS, core-engine |
| #8 | M5 — Tauri 2 masaüstü, Expo mobil, ortak `trading-core` paketi |
| #9 | Terminal ↔ client-gateway gerçek protokol + canlı e2e |
| #10 | Çekirdek entegrasyonu: gateway → core-engine (risk/OMS/defter) → FIX LP, gerçek bakiye ve K/Z |
| #12 | Kimlik servisi: kayıt/giriş, 2FA, passkey, dönen oturumlar, JWKS; terminal girişi |
| #13 | Back office ↔ core-engine admin API (JWT RBAC, 4-göz, idempotency, denetim, SSE) |
| #14 | Yük testi aracı, güvenlik incelemesi (17 bulgu), README/mimari |
| #15 | Güvenlik düzeltmeleri G1–G13, G16 |

## Yarım kalan işler

1. **PR #11 — protokol v1.2 (tüm emir tipleri)** — dal `feat/full-order-types`.
   Stop/stop-limit, SL/TP/trailing, hedging pozisyonları, kısmi kapanış, OCO, GTD, işlem geçmişi.
   Önceki commit CI'da tamamen yeşildi; son commit (`b025de8`) #15'i birleştiriyor ve yerelde
   doğrulanmadı. **Yapılacak:** CI sonucuna bak, yeşilse birleştir; kırmızıysa #15 ile çakışan
   client-gateway `conn.rs`/`hub.rs`/`main.rs` noktalarını düzelt.
2. **`feat/native-integration`** (PR yok) — terminal ↔ Tauri köprüsü (grafik pencere ayırma,
   tepsi durumu, yerel bildirimler) ve mobil canlı WS modu. Commit `415ed56` **WIP, doğrulanmamış**.
   **Yapılacak:** `main` + #11 birleştir, testleri koş, PR aç.
3. **Güvenlik:** G14, G15, G17 ertelendi; G8, G10, G13 kısmi — bkz. `docs/07-guvenlik-duzeltmeleri.md`.
4. **Bilinen eksikler:**
   - LP oturumları ve işlem geçmişi admin API'de boş.
   - NATS tabanlı `CoreApi` yok.
   - Gerçek LMAX FIX spesifikasyonu henüz alınmadı; LMAX'e özgü ayrıntılar `[DOĞRULA]` işaretli.
   - İmzalı masaüstü sürümü ve updater anahtarları yok.

## Elle yapılması gerekenler (aracın yetkisi dışında)

- **GitHub Pages:** Settings → Pages → Source: **GitHub Actions**. Açılınca `main`'e her push terminali
  (kök) ve back office'i (`/admin/`) yayınlar. Depo private ise ücretli plan gerekir.
- **Depo görünürlüğü:** depo public açıldı; private yapmak için Settings → Danger Zone.

## Hızlı başlangıç

```bash
cargo run -p client-gateway -- --demo        # simülatör + FIX + çekirdek, demo token stdout'ta
cd apps/terminal && pnpm install --ignore-workspace && pnpm dev   # ?api=ws&url=...&token=...
```

Ayrıntılar: kök `README.md`, `docs/04-operasyon.md`, `docs/05-performans.md`.
