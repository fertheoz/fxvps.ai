# Brifing — fxvps.ai LP konsolu (LMAX demo) için ödünç LXC — sürüm 2

**Kimden:** fxvps.ai projesi (bulut oturumu, yalnız kod).
**Kime:** mt5forexvps masaüstü oturumu (`/root/mt5forexvps`).
**Onaylayan:** Kurucu.
**Süre:** Geçici, LMAX demo bağlantı testi bitene kadar.

**v1'e göre değişenler:**
- Kimlik CT 970.
- Genel IP yok; NAT arkasında özel IP yeterli.
- `lmax-demo.toml` kalktı. LP bilgileri artık kurucu tarafından **konsoldan** girilir.
- Kutuda üç kap çalışır: fix-gateway, core-engine, konsol. Hepsi CT içinde yalnız 127.0.0.1'de.

## Mevcut CT 970'i yükseltme (5 Ekim'de kuruldu, 104.243.32.199)
CT hazır olduğu için 1–2. adımlar atlanır. Sırayla:
1. **Durdur ve güncelle:**
   - `cd /opt/fxvps.ai/deploy/lmax-demo && docker compose down`. Daha önce hiç `up` yapılmadıysa zararsız.
   - `git -C /opt/fxvps.ai pull`
2. **`.env`'i yenile:**
   - Eski `FIX_GATEWAY_*_PASSWORD` satırları artık kullanılmıyor.
   - `cp .env.example .env.new`, `FIX_ADMIN_TOKEN=$(openssl rand -hex 24)` değerini yaz, `mv .env.new .env && chmod 600 .env`.
3. **Veri dizini:** `mkdir -p core-data && chown 65532:65532 core-data`. `fix-store` zaten var.
4. **SSH:**
   - `apt-get install -y openssh-server`, kurucunun açık anahtarı, `PasswordAuthentication no`.
   - CT genel IP'de olduğu için **yalnız anahtarla giriş**. Root parola girişi kapalı kalmalı.
5. **Çalıştır:** `docker compose pull && docker compose up -d`, sonra 9. adımdaki doğrulamalar.
6. **Kurucuya tünel komutu:** genel IP olduğu için atlama noktası gerekmez:
   `ssh -L 8080:127.0.0.1:8080 root@104.243.32.199` → tarayıcıda `http://localhost:8080`.
7. **Dışarıya açık port kontrolü:** `ss -ltnp` çıktısında 22 dışında `0.0.0.0`/genel IP'de dinleyen bir şey olmamalı. 8080, 8090 ve 9890 yalnız 127.0.0.1'de.

## Amaç
fxvps.ai'nin LP konsolunu ayağa kaldırmak. Kurucu LMAX demo FIX 4.4 bilgilerini konsoldaki LP sayfasına girer. Gateway oturumları açar, durum aynı sayfada görünür. Bu bir **bağlantı testi**: gerçek para yok, müşteri verisi yok, mt5forexvps ürününe dokunan değişiklik yok.

## Kapsam sınırları (mt5forexvps kuralları geçerli)
- **Müşteri kutularına** (LXC 1xx, VM 106) ve **golden şablonlara** (11xx, CT 980 golden soyu, onaylı şablon mührü) dokunulmaz. 966 (kapı) dokunulmaz.
- mt5 üretim kaplarına ve `/opt/mt5forexvps`'e dokunulmaz.
- Hepsi **CT 970** (`fxvps-lmax-demo`) içinde çalışır.
- Hiçbir şey "temizlik" diye silinmez. İş bitince CT durdurulur; silmeye kurucu karar verir.
- LP parolasını oturum görmez, istemez, basmaz. Kurucu konsola kendisi girer.

## Kaynak ve ağ
| | |
|---|---|
| CT | 970 · Debian 13 standart şablon · 1 vCPU / 1 GB / 8 GB · `nesting=1` · `onboot=0` |
| Ağ | NAT arkasında özel IP. Genel IP ve port yönlendirme **yok** |
| Dışarı çıkış | GHCR (imaj çekme, HTTPS) ve LMAX demo FIX host'ları (TLS). Bunlar kurucu konsola girince belli olur |
| İçeri giriş | Yalnız SSH (kurucunun anahtarı), konsol tüneli için |

## Adımlar
1. **Ön kontrol (salt okuma):** CT 970 hâlâ boş mu, seçilen özel IP çakışıyor mu? Çakışma varsa dur, kurucuya sor.
2. **CT 970'i oluştur** (yukarıdaki değerlerle) ve başlat.
3. **CT içinde:**
   - `apt-get install -y docker.io docker-compose-plugin git openssh-server`. Paket adı sürüme göre `docker-compose` olabilir.
   - Kurucunun açık anahtarını `/root/.ssh/authorized_keys`'e ekle. `PasswordAuthentication no`.
4. **Paketi getir:** `git clone https://github.com/fertheoz/fxvps.ai /opt/fxvps.ai && cd /opt/fxvps.ai/deploy/lmax-demo`.
5. **Sır:**
   - `cp .env.example .env && chmod 600 .env`.
   - `FIX_ADMIN_TOKEN` değerini `openssl rand -hex 24` ile üret ve yaz. Bu bir LP parolası değil, iki kap arasındaki iç anahtar. Oturum değeri ekrana basmaz.
6. **Veri dizinleri:** `mkdir -p fix-store core-data && chown 65532:65532 fix-store core-data`.
7. **İmajlar:** `docker compose pull`. İmajlar public; jeton ve derleme gerekmez.
8. **Çalıştır:** `docker compose up -d`, ardından `docker compose ps` ile üç kabın da `running` olduğunu gör.
9. **Doğrula (CT içinde, salt okuma):**
   - `curl -s 127.0.0.1:8080/healthz` → `ok`
   - `curl -s 127.0.0.1:9890/status` → `[]`, çünkü henüz LP girilmedi.
   - `docker compose logs fix-gateway | tail` → `no LP configured yet; waiting for PUT /config`.
10. **Kurucuya tünel komutunu ver.** CT özel IP'si `<CT_IP>`, Proxmox düğümü atlama noktası:
    ```
    ssh -J root@104.194.9.154 -L 8080:127.0.0.1:8080 root@<CT_IP>
    ```
    Kurucu tarayıcıda `http://localhost:8080` açar. Girişte **"Dev token"** → rol **admin** seçer, sonra LP sayfasına gider ve LMAX bilgilerini girip kaydeder.
11. **Rapor:**
    - `curl -s 127.0.0.1:9890/status` çıktısı. Hedef: iki satır, `"logged_on": true`.
    - `docker compose logs --tail=200 fix-gateway` içinden parolasız hata satırları.
    - fxvps.ai'ye iletilir.

**Emir gönderme yok.** Bu kurulum yalnız bağlantı ve fiyat testi.

## Sorun giderme
| Belirti | Olası neden |
|---|---|
| Konsol açılmıyor | Tünel yok ya da `console` kabı ayakta değil (`docker compose logs console`) |
| Konsolda LP ayar kartı "yönetilmiyor" diyor | core-engine `CORE_LP_ADMIN_URL`/token eşleşmiyor → `.env`'deki `FIX_ADMIN_TOKEN` |
| Kaydet → hata "host:port", "TLS" | Girilen değerde biçim hatası. Mesaj konsolda görünür |
| `connect ...: refused/timed out` | Yanlış host/port ya da CT'den dışarı çıkış engeli |
| TLS hatası (`UnknownIssuer`, `invalid peer certificate`) | LMAX özel CA ya da farklı sunucu adı. fxvps.ai'ye bildir |
| Logon sonrası hemen `Logout` | CompID, kullanıcı ya da parola hatalı; ya da sequence reset politikası |

## Bitirme / geri alma
- `docker compose down` (CT içinde), sonra `pct stop 970`.
- CT silme kurucu kararıdır. `fix-store/lp.json` LP parolasını içerir; CT silinmeyecekse bu dosyayı ayrıca kaldırmayı kurucuya sor.

## Hafıza
`docs/agent-memory/` altına not: CT 970, özel IP, tarih, sonuç (parolasız). ACIK-ISLER'e "fxvps LP konsolu CT 970 — geçici, sökülecek".
