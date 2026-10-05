# Brifing v4 (KESİN) — fxvps.ai demo platformu: konsol + müşteri terminali + LMAX demo (CT 970)

**Kimden:** fxvps.ai projesi · **Kime:** mt5forexvps masaüstü oturumu · **Onaylayan:** Kurucu
**v1–v3'ün yerine geçer.** Çelişki varsa bu belge geçerlidir.

---

## 0. Ne değişti, neden
- **Tek işlem süreci:** müşteri emirleri için çekirdek ve FIX aynı süreçte olmalı. LMAX de CompID başına tek oturuma izin veriyor. Bu yüzden ayrı `fix-gateway` ve `core-engine` kapları kalktı; yerlerine tek **`trading`** kabı geldi (client-gateway + core-engine + fix-gateway).
- **Konsol girişi:** artık **e-posta + parola + 2FA** ile, kimlik servisinden yapılıyor. Kurucu isteği: "admin şifresi olmadan girilmesin". Cloudflare Access dış kapı olarak kalır; dev-token ve Access'ten otomatik giriş kalktı.
- **Yeni yayınlar:** müşteri terminali **`trade.fxvps.ai`**, kimlik servisi **`id.fxvps.ai`**. İkisi de herkese açık; korumayı kimlik girişi sağlar.
- **LMAX miktar düzeltmesi (#38):** 1 lot artık 10 LMAX kontratı olarak gidiyor. Önceki kodla 100.000 kontrat gidecekti. Bu düzeltme olmadan emir verilmez.

| Yerel adres (CT içinde) | Servis | Yayın |
|---|---|---|
| 127.0.0.1:8080 | Konsol (nginx → admin API) | `console.fxvps.ai` + **Access** |
| 127.0.0.1:8081 | Kimlik servisi | `id.fxvps.ai` |
| 127.0.0.1:8082 | Müşteri terminali (nginx → `/ws`) | `trade.fxvps.ai` |
| 127.0.0.1:8088 | trading: müşteri WebSocket | (terminal üzerinden) |
| 127.0.0.1:8090 | trading: admin API | (konsol üzerinden) |
| 127.0.0.1:9890 | trading: LP durum / LP ayar ucu (token'lı) | — |
| 127.0.0.1:5433 | PostgreSQL (kimlik) | — |

## 1. Kim ne yapar
| Adım | Yapan |
|---|---|
| Cloudflare: 3 public hostname + Access (yalnız console) | **Kurucu** (panelde) |
| CT 970: dosyalar, `.env`, anahtar, kaplar, doğrulama, operatör komutları | **mt5forexvps oturumu** |
| Konsola giriş, LP bilgileri (LMAX parolası), Saygın'ın hesabını açma ve bakiye yükleme | **Kurucu** |
| Kod hatası, yeni imaj | **fxvps.ai** |

Sırlar (`.env` değerleri, imza anahtarı, LMAX parolası) hiçbir yere yazılmaz, ekrana basılmaz.

---

## 2. Kurucu — Cloudflare (mevcut `fxvps-console` tünelinde)
Zero Trust → Networks → Tunnels → `fxvps-console` → **Public Hostname**. Üç kayıt; hepsinde Type **HTTP**:

| Subdomain | Domain | URL |
|---|---|---|
| `console` | `fxvps.ai` | `127.0.0.1:8080` (varsa dokunma) |
| `trade` | `fxvps.ai` | `127.0.0.1:8082` |
| `id` | `fxvps.ai` | `127.0.0.1:8081` |

- **Access uygulaması yalnız `console.fxvps.ai` içindir.** `trade` ve `id` için Access **yok**: müşteri ve kimlik servisi herkese açık olmalı.
- DNS'e elle kayıt eklenmez; tünel CNAME'leri kendisi oluşturur.

## 3. mt5forexvps — CT 970
```bash
cd /opt/fxvps.ai && git pull                                   # bu brifingi içeren main
cd deploy/lmax-demo
docker compose --profile tunnel down                           # eski kaplar (fix-gateway, core-engine, console, cloudflared)
cp .env .env.yedek-$(date +%Y%m%d%H%M)
mkdir -p fix-store core-data pg-data keys
chown 65532:65532 fix-store core-data
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out keys/identity.pem
chown 65532:65532 keys/identity.pem && chmod 600 keys/identity.pem
```
`.env` (mevcut `FIX_ADMIN_TOKEN` ve `CLOUDFLARE_TUNNEL_TOKEN` korunur; `CORE_DEV_AUTH` ve `CF_ACCESS_*` satırları silinir):
```
FIX_ADMIN_TOKEN=<mevcut>
PG_PASSWORD=<openssl rand -hex 24>
CLOUDFLARE_TUNNEL_TOKEN=<mevcut>
FIXVPS_TAG=latest
RUST_LOG=info
```
```bash
chmod 600 .env
docker compose --profile tunnel pull
docker compose --profile tunnel up -d
docker compose --profile tunnel ps     # postgres, identity, trading, console, terminal, cloudflared → running
```
Eski LP ayarı (`fix-store/lp.json`) korunur; trading kabı onu okur. **Not:** dosyadaki enstrüman satırlarında kontrat büyüklüğü yok, yani 1 = birim. Kurucu konsolda kaydetmeden emir verilmeyecek (§5).

### 3.1 Doğrulama
| # | Komut | Beklenen |
|---|---|---|
| 1 | `curl -s 127.0.0.1:8081/.well-known/jwks.json \| head -c 80` | `{"keys":[` |
| 2 | `curl -s -o /dev/null -w '%{http_code}\n' 127.0.0.1:8090/v1/me` | `401` |
| 3 | `curl -s -o /dev/null -w '%{http_code}\n' -XPOST 127.0.0.1:8090/auth/dev-token -H 'content-type: application/json' -d '{"role":"admin"}'` | `404` |
| 4 | `curl -s 127.0.0.1:9890/status` | LP satırları (LMAX bilgileri girildiyse) ya da `[]` |
| 5 | `curl -s 127.0.0.1:8082/healthz; curl -s 127.0.0.1:8080/healthz` | `ok` `ok` |
| 6 | `ss -ltnp` | 0.0.0.0/genel IP'de yalnız 22 |
| 7 | (CT dışından) `curl -sI https://trade.fxvps.ai \| head -1` ve `curl -s https://id.fxvps.ai/.well-known/openid-configuration \| head -c 60` | `200` ve `{"issuer":"https://id.fxvps.ai"` |
| 8 | (CT dışından) `curl -sI https://console.fxvps.ai \| head -3` | Access yönlendirmesi (302) |

## 4. Kurucunun admin hesabı (bir kerelik)
1. Kurucu `https://trade.fxvps.ai` → **Kayıt ol** → `ferthe@gmail.com` + güçlü parola.
2. mt5 oturumu (CT içinde):
   ```bash
   docker compose exec identity /app/identity verify-email ferthe@gmail.com
   docker compose exec identity /app/identity grant-admin ferthe@gmail.com
   ```
3. Kurucu `trade.fxvps.ai`'ye giriş yapar → hesap menüsünden **2FA (TOTP)** kurar (Google Authenticator vb.) → kurtarma kodlarını saklar.
4. Kurucu `https://console.fxvps.ai` → Access e-posta kodu → konsol girişi: e-posta + parola + 2FA kodu.
   *Para ve ayar değiştiren işlemler 2FA'lı giriş ister (`CORE_REQUIRE_MFA=1`).*

## 5. Kurucu — LP (LMAX) ayarı
Konsol → **LP** → **LP bağlantı ayarları**:

| Alan | Piyasa verisi | İşlem |
|---|---|---|
| Host:port | `fix-marketdata.london-demo.lmax.com:443` | `fix-order.london-demo.lmax.com:443` |
| SenderCompID / Kullanıcı | `FXVPS-Ferdi` | `FXVPS-Ferdi` |
| TargetCompID | `LMXBDM` | `LMXBD` |
| Parola | LMAX FIX parolası | aynı |
| TLS | on | on |

- **Enstrümanlar**, satır başına `SEMBOL SecurityID tick kontrat` (LMAX FX kontratı = **10000**):
  ```
  EUR/USD 4001 0.00001 10000
  GBP/USD 4002 0.00001 10000
  USD/JPY 4004 0.001 10000
  ```
- **Tam liste** (84 FX paritesi + 7 değerli maden: altın, gümüş, platin, paladyum): `deploy/lmax-demo/lmax-instruments.txt` içeriğini kutuya yapıştırın. Satırlar `SEMBOL ID tick kontrat` biçiminde.
- Diğer alanlar: LP `LMAX` · Heartbeat `30` · SecurityIDSource `8`.
- **Kaydet** → trading kabı yeniden başlar (yaklaşık 5–10 sn) → tabloda MD ve TRADING satırları **logged_on** olur.

## 6. Saygın Balıkel — 5.000 USD demo hesabı
1. **Hesap (konsol):** Müşteriler → **Hesap aç** → Ad `Saygın Balıkel`, e-posta `saygin.balikel@gmail.com`, grup **`demo-retail`** (USD, 1:30, A-book → LMAX) → hesap numarası görünür (ör. **100001**).
2. **Bakiye (konsol):** hesabı aç → **Para yatırma** → `5000` USD, gerekçe `demo funding`. 4-göz eşiği aşılırsa ikinci onay istenir. Tek admin varsa eşik Ayarlar'dan geçici olarak yükseltilir.
3. **Giriş:** Saygın `https://trade.fxvps.ai` → **Kayıt ol** (`saygin.balikel@gmail.com`).
4. **Bağlama (mt5 oturumu):**
   ```bash
   docker compose exec identity /app/identity verify-email saygin.balikel@gmail.com
   docker compose exec identity /app/identity link-account saygin.balikel@gmail.com <hesap-no>
   ```
5. Saygın tekrar giriş yapar → terminalde **5.000 USD** bakiyeli hesabı görür, fiyatlar LMAX demodan akar.
   - İlk işlemi **0.01 lot**, yani LMAX'te 0.1 kontrat (en küçük emir) olsun.
   - Doğrulama: konsol → Pozisyonlar'da görünür; `docker compose logs trading | grep -i exec` satırlarında LMAX dolumu (`35=8`) görünür.

## 7. Sorun giderme
| Belirti | Çözüm |
|---|---|
| trade/id → 522 ya da 1033 | §2 hostname'leri; `docker compose logs cloudflared` |
| Konsol "invalid token" ya da 401 | `CORE_JWT_ISSUER` = `https://id.fxvps.ai` ve kimlik servisi ayakta mı (#1) |
| Konsolda değişiklik "mfa required" | Kurucu 2FA kurmadan girmiş → §4.3 |
| Terminal girişte "Origin not allowed" | `IDENTITY_ALLOWED_ORIGINS` / `FXVPS_ALLOWED_ORIGINS` |
| Saygın hesabı göremiyor | §6.4 bağlama + çıkış yapıp tekrar giriş |
| Emir "no LP mapping" ya da reddedildi | §5 enstrüman satırları ve kontrat sütunu |
| Logon sonrası `Logout` / TLS hatası | `docker compose logs --tail=200 trading` → parolasız satırları fxvps.ai'ye ilet |

## 8. Geri alma (hiçbir şey silinmez)
- **Yayını kapat:** `docker compose stop cloudflared`.
- **Platformu durdur:** `docker compose --profile tunnel down`. Veri dizinleri (`fix-store`, `core-data`, `pg-data`, `keys`) diskte kalır.
- Hostname, Access, DNS, CT silme ve IP iadesi kurucu kararıdır.

## 9. Hafıza
`docs/agent-memory/`: tarih, CT 970, hostnames, Saygın hesap no (parolasız), doğrulama sonuçları. ACIK-ISLER: "fxvps demo platformu CT 970 — geçici, sökülecek".
