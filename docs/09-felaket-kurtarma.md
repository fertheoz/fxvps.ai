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

## Yapılmayan / sonraya
* **Mavi/yeşil (sıfır kesintili) devir:** LMAX tek CompID ile iki eşzamanlı oturum kabul etmez; devir için ya ikinci CompID ya da FIX oturumunu süreçler arası taşıyan ayrı fix-gateway süreci (Etap 14, NATS ayrımı) gerekir. Bugün dağıtım ≈ 30–45 sn kopma.
* Offsite (başka veri merkezi) kopya: `rclone` hedefi tanımlanınca `yedek.sh` sonuna tek satır.
