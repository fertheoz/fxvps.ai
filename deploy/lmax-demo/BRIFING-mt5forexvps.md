# Brifing — fxvps.ai için LMAX demo FIX bağlantısı (mt5forexvps altyapısında ödünç LXC)

**Kimden:** fxvps.ai projesi (bulut oturumu, yalnız kod).
**Kime:** mt5forexvps masaüstü oturumu (`/root/mt5forexvps`).
**Onaylayan:** Kurucu.
**Süre:** Geçici, bağlantı testi bitene kadar (birkaç gün–birkaç hafta).

## Amaç
fxvps.ai'nin `fix-gateway` servisini (tek Rust ikilisi, Docker imajı) LMAX **demo** ortamına FIX 4.4 ile bağlamak. MD ve trading oturumlarının logon olduğunu görmek ve fiyatların geldiğini doğrulamak. Bu bir **bağlantı testi**. Gerçek para yok, müşteri verisi yok, mt5forexvps ürününe dokunan bir değişiklik yok.

## Kapsam sınırları (mt5forexvps kuralları geçerli)
- **Müşteri kutularına** (LXC 1xx, VM 106) ve **golden şablonlara** (11xx, onaylı şablon mührü) dokunulmaz.
- mt5 üretim kaplarına (`mt5forexvps-api-1`, `-web-1`, `-postgres-1`) ve `/opt/mt5forexvps`'e dokunulmaz. Bunların docker compose'u kullanılmaz.
- fix-gateway **ayrı bir LXC'de** çalışır. Kimlik önerisi: **CT 980** (boşsa; 9xx aralığı, 966 kapı için ayrılmış, ona dokunma). Ad: `fxvps-lmax-demo`.
- Hiçbir şey "temizlik" diye silinmez. İş bitince CT durdurulur. Silmeye kurucu karar verir.
- LMAX parolası yalnız CT içindeki `.env` dosyasında durur (`chmod 600`). Log'a, bilete, Telegram'a, depoya yazılmaz, ekrana basılmaz.

## Gerekenler
| Kaynak | Değer |
|---|---|
| CT şablonu | Debian 12/13 standart LXC (golden değil) |
| vCPU / RAM / disk | 1 / 1 GB / 8 GB |
| Ağ | Boşta duran **genel IP**'lerden biri (kurucu seçer), varsayılan ağ geçidi |
| Özellikler | `nesting=1` (CT içinde Docker için) |
| Dışarı çıkış | LMAX demo FIX host:port'larına TCP (TLS); GHCR'ye HTTPS (imaj çekme) |
| İçeri giriş | **Yok.** Hiçbir port dışarı açılmaz. Durum ucu yalnız `127.0.0.1:9890` |

> LMAX çoğu zaman bağlanan IP'yi beyaz listeye alır. Seçilen IP'yi kurucu LMAX'e bildirir. Logon "connection refused/reset" ile düşüyorsa ilk şüpheli budur.

## Adımlar
1. **Ön kontrol (salt okuma):** CT 980 boş mu, seçilen IP başka bir kutuda kullanılıyor mu (`pct list`, `pct config <id>` ile IP taraması, düğümde `ip neigh`)? Çakışma varsa dur ve kurucuya sor.
2. **CT'yi oluştur:** Debian şablonundan CT 980. 1 vCPU, 1 GB RAM, 8 GB disk, `nesting=1`, seçilen genel IP/ağ geçidi, `onboot=0`. Başlat.
3. **CT içinde Docker:** `apt-get install -y docker.io docker-compose-plugin git` (paket adı sürüme göre `docker-compose` olabilir).
4. **Paketi getir:** `git clone https://github.com/fertheoz/fxvps.ai /opt/fxvps.ai` (depo public). Sonra `cd /opt/fxvps.ai/deploy/lmax-demo`.
5. **İmaj:** `docker pull ghcr.io/fertheoz/fxvps-fix-gateway:latest`. Çekme yetki hatası verirse (paket private) imajı yerelde derle: depo kökünde `docker build -f infra/docker/fix-gateway.Dockerfile -t ghcr.io/fertheoz/fxvps-fix-gateway:latest .` (Rust derlemesi; 1 vCPU'da uzun sürer, geçici olarak 2–4 vCPU verilebilir).
6. **Ayar:** `lmax-demo.toml` içindeki `<...>` alanlarını kurucunun verdiği LMAX demo FIX bilgileriyle doldur: host:port, SenderCompID/TargetCompID, kullanıcı adı. LMAX düz TCP diyorsa `[md.tls]` ve `[trade.tls]` tablolarını kaldır.
7. **Parola:** `cp .env.example .env && chmod 600 .env`. Kurucu parolayı **kendisi** girer (`FIX_GATEWAY_MD_PASSWORD`, `FIX_GATEWAY_TRADE_PASSWORD`). Oturum parolayı görmez, istemez, basmaz.
8. **Depo dizini:** `mkdir -p fix-store && chown 65532:65532 fix-store` (imaj distroless nonroot ile çalışır).
9. **Çalıştır:** `docker compose up -d`, sonra `docker compose logs -f --tail=200`.
10. **Doğrula:**
    - Loglarda MD ve trading için `logged on` / `SessionUp`.
    - `curl -s 127.0.0.1:9890/status` → iki satır, `"logged_on": true`.
    - Debug loglarda EUR/USD fiyat (`quote`) olayları akıyor.
11. **Emir gönderme yok.** Bu kurulum yalnız bağlantı ve fiyat testi. Emir testi ayrıca planlanacak.

## Sorun giderme
| Belirti | Olası neden |
|---|---|
| `connect ...: Connection refused/timed out` | IP beyaz listede değil, yanlış host/port ya da çıkış engeli |
| TLS hatası (`invalid peer certificate`, `UnknownIssuer`) | LMAX özel CA kullanıyor → `ca_file` ver; ya da `server_name` farklı |
| Logon sonrası hemen `Logout` | Yanlış CompID/kullanıcı/parola ya da sequence reset politikası (`reset_on_logon`) |
| Fiyat yok, oturum açık | SecurityID/IDSource eşlemesi (`security_id_source`, enstrüman ID'leri) |

Hata metnini (parola içermeyen satırlar) fxvps.ai tarafına ilet: kodu orada düzeltip yeni imaj çıkarırız.

## Bitirme / geri alma
- Durdur: `docker compose down` (CT içinde), sonra `pct stop 980`.
- CT'yi silmek ve IP'yi geri vermek kurucunun kararıdır.
- `/opt/fxvps.ai/deploy/lmax-demo/.env` CT ile birlikte gider. CT silinmeyecekse `.env`'i elle sil.

## Rapor
`docs/agent-memory/` altına kısa bir not düş: CT kimliği, IP, başlangıç tarihi, logon sonucu (parolasız). ACIK-ISLER'e "fxvps LMAX demo CT 980 — geçici, sökülecek" maddesini ekle.
