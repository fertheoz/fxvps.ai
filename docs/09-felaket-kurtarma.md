# Felaket kurtarma ve dağıtım kapısı (Etap 10)

Kapsam: CT 970 (`fxvps-lmax-demo`). Tek süreçli demo yığını; hedef "veri kaybı
yok, dağıtım geri alınabilir, yedek kanıtlı".

## Veri nerede
| Veri | Yol (CT) | Not |
|---|---|---|
| Motor journal + snapshot | `deploy/lmax-demo/core-data/{journal.jsonl,snapshot.json}` | tek gerçek; her komut journal'da |
| Admin defteri (denetim, bakiye işlemleri, ayarlar) | `core-data/admin.jsonl` | append-only |
| Mum geçmişi, müşteri tercihleri | `core-data/{candles.txt,prefs.json}` | yeniden üretilemez (LP geçmiş vermez) |
| LP bağlantısı (parola dahil) | `fix-store/lp.json` | 0600 |
| Identity (kullanıcılar, 2FA, oturumlar) | Postgres `identity` (`pg-data/`) | dump ile yedeklenir |
| İmza anahtarı, .env, compose | `keys/`, `.env`, `docker-compose*.yml` | |

## Yedek (`yedek.sh`, cron 03:30 UTC)
* Hepsini `tar.gz` → **AES-256-CBC (openssl, pbkdf2 200k)** → `/var/lib/fxvps-yedek/fxvps-<zaman>.tgz.enc` + sha256.
* `/var/lib/fxvps-yedek` CT'ye **Proxmox host'tan bağlanan** dizindir (`pct set 970 -mp0 /var/lib/vz/yedek/fxvps,mp=/var/lib/fxvps-yedek`): CT diski giderse yedek kalır.
* Anahtar: CT `/etc/fxvps/yedek.key` (0600) ve host `/root/fxvps-yedek.key`. **Anahtar olmadan yedek açılmaz**; kurucu kopyasını D:\secure'a alır. Anahtar hiçbir yere yazdırılmaz.
* **Prova her gün otomatik:** yedek çözülür, `core-engine verify` journal'ı replay eder ve değişmezleri denetler. Başarısızsa `yedek.durum` = `HATA …` → konsolda **"Backup / restore drill failed"** uyarısı (critical); 36 saattir başarılı yedek yoksa "stale" uyarısı. Sessiz yedek arızası yok.
* Saklama: 14 gün.

## Geri yükleme (`geri-yukle.sh <dosya> [--uygula]`)
1. Kuru koşum: çözer, replay eder, raporlar (uygulamaz).
2. `--uygula`: servisleri durdurur, `core-data` → `core-data.eski-<zaman>`, yedeği koyar, identity DB'yi `identity_restore` olarak kurup eskisiyle yer değiştirir, servisleri başlatır. Eski veri silinmez.
3. Yeni CT'ye kurulum: repo klonu + `.env`/`keys` yedekten + `geri-yukle.sh … --uygula`.

## Dağıtım kapısı (`dagit.sh [servis…]`)
1. İmajları çek.
2. **Replay kapısı:** yeni `core-engine` imajı bugünkü journal kopyasını replay edip değişmezleri geçmeli (şema/replay regresyonu burada kalır; çıkış 2).
3. Çalışan imajlar `geri-donus-<AAGG-SSDD>` etiketlenir ([[geri-donus-hedefi-etiketlenmeli]]).
4. `up -d`; 120 sn içinde en az bir FIX oturumu `logged_on` değilse **otomatik geri alma** (çıkış 3).
5. Kayıt: `deploy/lmax-demo/dagit.log`.

## Journal büyümesi
`CORE_JOURNAL_COMPACT_MB` (varsayılan 256): başlangıçta journal bu boyutu aşmışsa motor snapshot alır ve eski journal'ı `core-data/archive/journal-<seq>.jsonl` olarak kenara koyar (durum değişmez; `verify` digest'i aynı kalır). Elle: `core-engine compact --data-dir …` (yazıcı kapalıyken).

## Kesintisiz dağıtım (mavi/yeşil, 7 Eki)
* **Süreç ayrımı:** FIX oturumları `lp-gateway-lmax` / `lp-gateway-sim` konteynerlerinde (fix-gateway, `NATS_URL`). Trading çekirdeği LP'lerle **NATS** üzerinden konuşur: fiyatlar `fx.lp.<lp>.quotes`, icralar/retler JetStream `FXLP` (`fx.lp.<lp>.events`, 10 dk saklama; yeniden başlayan çekirdek son 120 sn'yi yeniden okur, motor exec id ile tekrarı ayıklar), durum `fx.lp.<lp>.status`, emirler istek/yanıt `fx.lp.<lp>.orders` (yanıtsız/zaman aşımı → emir reddi, sessiz kayıp yok). Trading yeniden başlarken **LMAX oturumu düşmez**.
* **LMAX freni:** gateway, trading oturumuna giden emirleri **saniyede 80** ile sınırlar (LMAX sınırı 100/sn); 250 ms'den fazla bekleyecek emir reddedilir (`max_orders_per_sec`, 0 = kapalı).
* **Mavi/yeşil:** `trading-blue` (WS 8088, admin 8090) ve `trading-green` (8089/8091) aynı imaj; `core-data/writer.lock`'u tutan (flock) hizmet verir, diğeri journal'ı **sıcak** tutar (replica). nginx upstream'leri kapalı portu atlar. `./dagit.sh trading`: bekleyen renk yeni imajla ısınır → aktif renk SIGTERM (son snapshot, kilit bırakılır) → bekleyen devralır (ölçülen süre `dagit.log`'da `DEVİR OK … ms`) → eski renk yeni imajla yedek olur. 60 sn'de devralma yoksa eski imaj geri gelir. Terminal 300 ms'de yeniden bağlanır.
* **Açılış hızı:** snapshot öncesi journal satırları tam ayrıştırılmadan atlanır (`{"seq":N` öneki) → yeniden başlatma ve ısınma saniyeler mertebesinde.
* Tek seferlik geçiş: `gecis-bluegreen.sh` (eski tek `trading`'den; bir kez ~30-60 sn kesinti).
* Statik konsol/terminal nginx'i güncellenirken (yeni ön yüz imajı) WS ~1 sn kopar; terminal kendiliğinden yeniden bağlanır.

## Gece yük sınavı (`yuk-sinavi.sh`, cron 03:50 UTC)
* Ayrı, geçici `client-gateway --demo` örneği (127.0.0.1:18088, kendi simülasyon LP'si, /tmp journal) — **canlıya ve LMAX'e dokunmaz**; CPU sınırlı (sunucu 2, loadgen 1 çekirdek).
* 1.000 eşzamanlı bağlantı, dakikada 10.000 emir, 120 sn. Ölçüt: bağlantı ≥ %99, emir hızı ≥ %95 hedef, yanıt ≥ %99, ret < %1, yanıt p99 < 250 ms.
* Sonuç `/var/lib/fxvps-yedek/yuk-sinavi/` (+`latest.json`), durum `yuk.durum` → dashboard "Gece yük sınavı" satırı, düşerse uyarı.
* Offsite (başka veri merkezi) kopya: `rclone` hedefi tanımlanınca `yedek.sh` sonuna tek satır.
