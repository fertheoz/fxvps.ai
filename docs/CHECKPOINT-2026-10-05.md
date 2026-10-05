# Checkpoint — 5 Ekim 2026

Yeni bir oturum (yerel ya da bulut) buradan başlar. Önceki özet: [`CHECKPOINT-2026-10-04.md`](CHECKPOINT-2026-10-04.md).
Sabit nokta: `main` @ `4def12d` (#48). Son güncelleme: 5 Ekim akşamı (bulut oturumundan yerele geçiş).

## 4–5 Ekim'de `main`'e birleşenler

| PR | İçerik |
|---|---|
| #24–#26 | G15 CORS, işlem geçmişi raporu, admin API'de LP oturum tablosu |
| #27 | G14 `identity grant-admin` CLI, G13 terminal CSP |
| #28 | G17 imaj digest sabitleme, SBOM, provenance, cosign |
| #30 | Terminal `packages/trading-core`'u kullanıyor (kopyalar silindi) |
| #31 | LP durumu ayrı süreçte (`FIX_STATUS_ADDR` / `CORE_LP_STATUS_URL`) |
| #32 | fix-gateway → LP **TLS** (`[md.tls]` / `[trade.tls]`) |
| #33–#34, #36 | `deploy/lmax-demo` kurulum paketi + mt5forexvps brifingleri (CT 970) |
| #35 | **LP konsolu:** LP FIX ayarı back office'ten (yönetilen config, parola yalnız yazılır, denetim kaydı) |
| #37 | Cloudflare Access girişi (core-engine `CORE_CF_ACCESS_*`, artık kullanılmıyor, kod duruyor) |
| #38 | **Kritik:** LP `OrderQty` kontrat cinsinden (`Instrument.contract_size`; LMAX FX 1 lot = 10 kontrat) |
| #39 | **İşlem altyapısı:** client-gateway = core + FIX + admin API tek süreçte; konsoldan **hesap açma** |
| #40 | **Demo platformu:** kimlik girişi (e-posta + parola + 2FA) konsol ve terminalde; `trade` / `id` hostname'leri; brifing v4 |
| #41 | Madenler (lot = ons), sonradan eklenen enstrümanlar motora; `lmax-instruments.txt` (84 FX + 7 maden) |
| #42 | Kimlik servisi SMTP'si `.env`'den (Resend örneği) |
| #43–#44 | Checkpoint; konsolda 2FA yönlendirmesi (ham `mfa_required` yerine) |
| #45 | **LP Stop/Play** + üst üste 3 başarısız logon'da devre kesici (LMAX hesap kilidi önlemi), `lastError` |
| #46 | `lmax-instruments.txt` testi: 91 satır, kontrat, 0.01 lot ≥ LMAX asgarisi |
| #47 | Konsol: kimlik kipinde yalnız e-posta/parola girişi |
| #48 | Terminal: jeton süresi dolunca (4001) yenilenmiş jetonla yeniden bağlanır |

## Canlı demo (CT 970, mt5forexvps altyapısında ödünç)

| Adres | Ne | Koruma |
|---|---|---|
| `https://console.fxvps.ai` | Back office (LP ayarı, hesaplar, bakiye) | Cloudflare Access (ferthe@gmail.com) + kimlik girişi, admin + 2FA |
| `https://trade.fxvps.ai` | Müşteri terminali | Kimlik girişi (kayıt açık) |
| `https://id.fxvps.ai` | Kimlik servisi | — |

- Kurulum ve işletim: `deploy/lmax-demo/` → `BRIFING-mt5forexvps.md` (v4, kesin), `docker-compose.yml`, `.env.example`.
- 6 kap: postgres, identity, trading (client-gateway + core + FIX + admin API), console, terminal, cloudflared. Hepsi 127.0.0.1'de, yayın Cloudflare Tunnel `fxvps-console` ile. İçeri yalnız 22 açık.
- **Kurallar:** CT ve prod sunucu işlemlerini **mt5forexvps masaüstü oturumu** yapar; fxvps.ai oturumu yalnız kod + PR yapar. Sırlar (LMAX parolası, tünel jetonu, SMTP anahtarı) sohbete ve depoya yazılmaz.
- LMAX demo: `fix-marketdata.london-demo.lmax.com:443` / `fix-order.london-demo.lmax.com:443`, Sender `FXVPS-Ferdi`, Target `LMXBDM` / `LMXBD`, TLS ≥ 1.2, beyaz liste yok. Parola LMAX'in güvenli bağlantısında.

## Durum (5 Ekim akşamı)
- LP: MD + TRADING LMAX demoya **logged_on**, fiyatlar trade.fxvps.ai'de akıyor.
- Kurucu hesabı 100001 (5.000 USD) bağlı, terminalden giriş tamam.
- İlk 0.01 lot emir **reddedildi**: sebep LMAX değil, terminal jetonu dolunca kopuk kalıyordu (#48 ile düzeldi). #45–#48 imajları CT 970'e henüz inmedi → `deploy/lmax-demo/BRIFING-v6-guncelleme.md` mt5'e verildi.
- Terminal bir kez "Simulator"da açıldı; geçici çözüm `https://trade.fxvps.ai/?api=ws&url=wss://trade.fxvps.ai/ws`. Kök sebep bakılacak.

## Açık işler (sırayla)

0. **v6 sonrası ilk gerçek emir:** EUR/USD 0.01 al/kapat → LMAX dolumu (`35=8`). Reddedilirse Journal + `docker compose logs trading` satırları.
0b. Terminal açılışta neden simülatöre düştü (sessionStorage / imaj `VITE_DEFAULT_WS_URL`).
0c. EURDKK / EURILS fiyat seyrek ("…") — LMAX ID'leri doğrula.

1. **SMTP (Resend):** Resend'e `fxvps.ai` alanını ekle, DNS kayıtlarını Cloudflare'e gir, API anahtarı oluştur. mt5 CT `.env`'e `IDENTITY_SMTP_URL=smtps://resend:<anahtar>@smtp.resend.com:465` ve `IDENTITY_MAIL_FROM=fxvps.ai <noreply@fxvps.ai>` yazar, `docker compose up -d identity`.
2. **Kurucu admin:** ferthe@gmail.com trade.fxvps.ai'de **kayıt oldu**. mt5: `docker compose exec identity /app/identity verify-email ferthe@gmail.com` ve `grant-admin`. Sonra terminalde 2FA (TOTP).
3. **LP ayarı (konsol → LP):** brifing §5 değerleri, enstrümanlar `deploy/lmax-demo/lmax-instruments.txt`. #41 imajları çekildikten sonra kaydet. Hedef: MD + TRADING `logged_on`.
4. **Test emri:** kurucu kendine bir hesap açıp terminalden 0.01 lot al/kapat. Bu, gerçek LMAX'e karşı ilk uçtan uca test (simülatörle test edildi). Hata çıkarsa `docker compose logs trading` → parolasız satırlar.
5. **Saygın Balıkel:** konsol → Hesap aç (`saygin.balikel@gmail.com`, grup `demo-retail`) → 5.000 USD yatır → Saygın trade.fxvps.ai'de kayıt olur → mt5 `link-account <e-posta> <hesap-no>` (SMTP yoksa önce `verify-email`).
6. ~~Konsol token kutuları~~ (#47). Cloudflare `*.fxvps.ai` joker A kaydı (185.53.179.128) kurucu kararı.

## Bilinen sınırlar
- Grafik geçmişi yok: LMAX FIX geçmiş mum vermiyor, grafikler çalışmaya başladığı andan itibaren dolar.
- LP ayarı değişince `trading` kabı yeniden başlar (5–10 sn).
- Admin token'ı 60 sn geçerli, konsol 30 sn'de bir yeniler (kimlik servisi çerezi).
- Önceki checkpoint'teki uzun vadeli işler sürüyor: NATS `CoreApi`, mobil EAS, masaüstü imzalı sürüm, performans (iki tepeli dolum gecikmesi), LMAX `[DOĞRULA]` maddeleri (`docs/02`).

## Yerelde başlamak
```bash
git clone https://github.com/fertheoz/fxvps.ai && cd fxvps.ai
claude        # ilk mesaj: "docs/CHECKPOINT-2026-10-05.md'yi oku, açık işlerden devam et"
```
Tarayıcıda iş (Resend, Cloudflare paneli) için Claude Code'un Chrome bağlantısı (`/chrome`) kullanılabilir.
