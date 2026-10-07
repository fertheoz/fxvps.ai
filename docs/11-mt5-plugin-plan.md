# 11 — MT5 plugin köprüsü: uygulama planı ve sınama senaryoları

Tasarım: [10-mt5-plugin-kopru.md](10-mt5-plugin-kopru.md). Bu belge işin
nasıl yapıldığını, neyin nerede durduğunu ve her senaryonun nasıl sınandığını
tutar. Durum sütunu her PR'da güncellenir.

## 0. Gerçekler ve kısıtlar

- **MT5 Server ve Server API SDK'sı indirilebilir değil.** MetaQuotes
  sunucuyu ve SDK'yı yalnız lisanslı kurumlara veriyor; bu makineye ya da
  sunucumuza "demo MT5 Server" kurulamaz. Bu yüzden plugin **ince bir
  adaptör** + **SDK'dan bağımsız çekirdek** olarak yazılır:
  - `plugin/core` — tüm iş mantığı (protokol, emir durum makinesi, çift
    gönderim koruması, kuyruk, düşüş modu, sembol/grup eşleme, mutabakat).
    Saf C++17, dış bağımlılık yok. Linux (gcc) ve Windows (MSVC) derlenir.
  - `plugin/host` — MT5'i soyutlayan arayüz (`IHost`): isteği onayla/reddet,
    tick bas, pozisyon netini oku, günlük yaz.
  - `plugin/mt5` — gerçek MT5 Server API adaptörü (`IHost` uygulaması).
    SDK gelene kadar derlemeye girmez (`FXVPS_MT5_SDK` kapalı); dosya, SDK
    çağrılarının nereye bağlanacağını işaretli iskelet olarak taşır.
  - `plugin/sim` — **sahte MT5 sunucusu** (`IHost` uygulaması): loginler,
    gruplar, istek kuyruğu, tick tablosu, net pozisyonlar. Sınamaların hepsi
    bunun üzerinden koşar.
- Taşıma: WebSocket üzerinden **JSON (protokol v1)**. Windows'ta WinHTTP
  (TLS dahil, sistem bileşeni) — DLL dış kütüphane taşımaz. Linux sınamaları
  için yalın TCP üzerinde küçük bir WebSocket istemcisi.

## 1. Protokol v1 (JSON metin çerçeveleri, alan `t` = tür)

| Yön | `t` | Alanlar |
|---|---|---|
| P→F | `hello` | `v`, `institution`, `key`, `server`, `plugin` |
| F→P | `welcome` | `v`, `account`, `heartbeat_ms`, `symbols[]` (`symbol`, `digits`, `contract_size`, `min_lot`, `lot_step`, `max_lot`) |
| F→P | `quote` | `s`, `b`, `a`, `ts` |
| P→F | `order` | `id` (kurumda tekil), `login`, `group`, `symbol`, `side` (`buy`/`sell`), `lots`, `kind` (`market`/`limit`), `price`, `deviation` (nokta) |
| F→P | `fill` | `id`, `lots` (bu parça), `price` (bu parça), `filled` (toplam), `avg`, `done` |
| F→P | `reject` | `id`, `code` (`price`, `liquidity`, `rate`, `session`, `bad_request`, `margin`, `closed`), `text` |
| P→F | `ping` / F→P `pong` | `ts` |
| P→F | `reconcile` | `net` {sembol: lot (işaretli)} |
| F→P | `reconcile_result` | `ok`, `ours` {sembol: lot}, `diff` {sembol: lot} |
| F→P | `error` | `code`, `text` (oturum düzeyi; ardından kapanış) |

Kurallar: tüm sayılar **ondalık metin** (kayan nokta yok). `id` yeniden
gönderilirse motor çift yürütmez, son durumu yeniden yollar. Kurum hesabı
**netting** grubunda olmalı (MT5'te kapanan pozisyon karşı yön emirle net
pozisyonu düşürür).

## 2. Bileşenler ve durum

| # | İş | Yer | Durum |
|---|---|---|---|
| A | Protokol belgesi + plan | `docs/10`, `docs/11` | ✅ |
| B | Köprü sunucusu `/bridge` (kurum kimliği, fiyat akışı, emir, dolum, mutabakat, kota) | `services/client-gateway/src/bridge.rs` | ✅ #127 |
| C | Kurum kayıtları (dosya; anahtar SHA-256 özetiyle) | `FXVPS_BRIDGE_FILE` | ✅ #127 |
| D | Plugin çekirdeği C++ | `plugin/core` | ✅ 89/89 kontrol (ASan+UBSan) |
| E | Sahte MT5 + senaryo koşucusu | `plugin/sim`, `plugin/tests` | ✅ |
| F | Uçtan uca: C++ plugin+sahte MT5 ↔ gerçek köprü ↔ demo motor | `plugin/tests/e2e-sunucu.sh` | ✅ E1 1000/1000, E2 112 yoldaki emir, mutabakat OK |
| G | WinHTTP taşıması + MT5 adaptör iskeleti (Windows derlemesi CI'da) | `plugin/win`, `plugin/mt5` | ✅ CI windows (MSVC /W4 /WX) |
| H | Canlıya: `wss://trade.fxvps.ai/bridge`, kurum dosyası, `kurum-ekle.sh` | CT 970 | ✅ #129 #130 (kurum: 0) |
| H2 | Windows'ta gerçek WinHTTP: canlı wss (TLS+kimlik reddi) + tünelle 200 emir | bu makine | ✅ 200/200 |
| H3 | Gece uçtan uca sınavı 04:10 UTC → `kopru.durum` | CT 970 cron | ✅ #131 |
| I | Konsol "Kurumlar" sayfası | backoffice | sonra |
| J | Gerçek MT5 adaptörü + test MT5 sunucusunda kabul | SDK gelince | bekliyor |

## 3. Sınama senaryoları (hepsi otomatik)

Çekirdek birim (sahte taşıma, saat elle):
1. **S1 Bağlanma:** hello → welcome; semboller eşlenir; yetkisiz grubun isteği plugin'de kalır (MT5'e "bize ait değil").
2. **S2 Piyasa emri tam dolum:** order → fill(done) → MT5 isteği fiyat+hacimle onaylanır.
3. **S3 Kısmi dolum:** iki fill parçası → MT5'e ortalama fiyatla tek onay (MT5 tek işlem görür).
4. **S4 Ret:** reject(price) → MT5 isteği "requote/off quotes" yerine kodla reddedilir.
5. **S5 Zaman aşımı:** yanıt 5 sn gelmez → istek "bekliyor" kalır, bağlantı yenilenince aynı `id` ile yeniden gönderilir; ikinci kez yürütülmez.
6. **S6 Kopukluk düşüş modu:** bağlantı yokken yeni istek → `reject` modunda hemen ret; `local` modunda MT5'e bırakılır (B-book).
7. **S7 Yeniden bağlanma:** artan bekleme (250 ms → 5 sn), iki uç arasında dönüşümlü (mavi/yeşil), açık istekler korunur.
8. **S8 Kalp atışı:** 3 sn sessizlik = kopuk sayılır.
9. **S9 Fiyat:** quote → MT5 tick; bayat fiyat (5 sn) işaretlenir; bilinmeyen sembol yok sayılır.
10. **S10 Mutabakat:** MT5 netleri gönderilir; fark varsa uyarı.
11. **S11 Sayı biçimi:** ondalık metin ↔ MT5 double dönüşümü basamakla yuvarlanır; 0.1+0.2 hatası yok.
12. **S12 Kötü girdi:** bozuk JSON / bilinmeyen tür → oturum kapanmaz, günlüğe düşer.

Sunucu (Rust, `cargo test`):
13. **K1 Kimlik:** yanlış anahtar → `error` + kapanış; doğru anahtar → welcome.
14. **K2 Emir/dolum:** demo motorda market emir → fill(done), avg doğru.
15. **K3 Çift gönderim:** aynı `id` iki kez → tek yürütme, ikinci yanıt aynı sonuç.
16. **K4 Kota:** saniyede N üstü → `reject(rate)`.
17. **K5 Netting:** buy 0.3 + sell 0.3 → kurum net 0.
18. **K6 Mutabakat:** plugin neti = bizim net → ok; farklı → diff.

Uçtan uca (CI, Linux):
19. **E1** C++ plugin + sahte MT5 (100 login, 3 grup) ↔ `client-gateway --demo` `/bridge`: 1.000 emir, hepsi onaylanır, MT5 netleri = kurum neti.
20. **E2** Emirler yoldayken bağlantı kesilir → plugin yeniden bağlanır, bekleyen emirler aynı kimlikle gönderilir, hiçbiri iki kez yürütülmez. (Demo motor diske yazmadığı için süreç yeniden başlatma burada sınanmaz; motorun journal'dan dönüşü mavi/yeşil devirde ölçüldü.)
21. **S15** Yoldaki emir varken mutabakat ertelenir (yanlış alarm yok).

## 4. Sonraki adımlar (SDK gelince)

- `plugin/mt5`: MT5 Server API'nin işlem isteği / onay ve tick arayüzlerine
  `IHost` bağlanır; plugin hiçbir MT5 geri çağrısında ağ beklemez (kuyruk).
- Test MT5 sunucusunda 3. bölümdeki senaryoların hepsi, sahte MT5 yerine
  gerçek sunucuyla tekrar koşulur (kabul sınavı).
- İmzalı DLL + kurulum kılavuzu.

## 5. İlk kurumu açmak (kurucu)

1. Konsolda kurum hesabını aç: **netting** grubunda, ön teminatlı.
2. Sunucuda: `./kurum-ekle.sh <kurum-id> <hesap> [emir/sn] [ip,...]` —
   anahtarı bir kez gösterir, trading'i kesintisiz yeniden yükler.
3. Kuruma: `plugin/mt5/fxvps-bridge.conf.example` + anahtar + kurulum notu.

## 6. Açık dış işler

- **MT5 Server API SDK + test sunucusu:** ilk kurumdan (bkz. §4).
- **LMAX pozisyon sorgusu yetkisi:** AN isteğine `PosReqResult=3`
  (yetkisiz) dönüyor. LMAX destekten hesap 663578833 için "FIX
  RequestForPositions" izni istenmeli (kurucu).
