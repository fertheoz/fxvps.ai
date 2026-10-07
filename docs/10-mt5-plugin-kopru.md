# 10 — MT5 Server plugin köprüsü (tasarım)

Karar (7 Eki 2026): aracı kurumlara likidite, oneZero / PrimeXM gibi üçüncü
taraf köprülerle değil, **kendi MT5 Server plugin'imizle** verilir. MT5
sunucusunun yöneticisi plugin'e grup yetkisi verir; o grupların emirleri
plugin üzerinden fxvps motoruna gelir, motor A-book (LMAX) / B-book
kurallarıyla yürütür, dolumlar MT5'e geri yazılır.

Ön koşul: MetaQuotes **Server API SDK**'sı ve bir test MT5 sunucusu. İkisi de
ilk aracı kurum müşterisinin lisansı altından gelir; kurucu MetaQuotes
lisansına kendisi girmez. SDK gelene kadar bu belgedeki "bizim taraf"
tamamen hazırlanır ve sahte bir MT5 istemcisiyle uçtan uca sınanır.

## 1. Bileşenler

```
 MT5 Server (kurum)                          fxvps
 ┌───────────────────────────┐   TLS + mTLS   ┌──────────────────────────┐
 │ fxvps plugin (C++ DLL)    │◄──────────────►│ bridge-gateway (Rust)    │
 │  · grup filtresi          │  köprü protok. │  · kurum kimliği / kota  │
 │  · emir yakalama/onay     │                │  · sembol eşleme         │
 │  · tick besleme           │                │  · idempotent emir alımı │
 │  · yerel kuyruk + düşüş   │                └─────────┬────────────────┘
 └───────────────────────────┘                          │ core API (mevcut)
                                              ┌─────────▼────────────────┐
                                              │ core-engine (OMS/risk)   │──► lp-gateway-lmax
                                              └──────────────────────────┘
```

- **Plugin (C++, Windows, MT5 Server API):** yetkili gruplardaki işlem
  isteklerini yakalar, bize iletir, gelen dolumla isteği onaylar / reddeder;
  bizden gelen fiyatları MT5 sembollerine tick olarak besler. Kendi içinde
  iş mantığı taşımaz (kurallar bizde) — ince istemci.
- **bridge-gateway (yeni servis, Rust):** client-gateway'in kardeşi. Kurum
  oturumlarını kabul eder; terminal yolundan farkı: tam hızlı fiyat (10 Hz
  sınırı yok), tek bağlantıda çok müşteri (MT5 login'leri), emir başına MT5
  istek kimliğiyle idempotentlik, kurum düzeyinde kota ve fren.
- **core-engine:** değişmez çekirdek. Her kurum bir **kurum hesabı**
  (omnibus) olarak görünür; MT5 login'i emirde alt-etiket olarak taşınır
  (raporlama ve toksik akış tespiti için), defter tutulmaz — müşteri
  defteri MT5'tedir.

## 2. Köprü protokolü

Taşıma: TLS üzerinden WebSocket, **JSON metin çerçeveleri (protokol v1)** —
aynı nginx, mavi/yeşil arkasında (`wss://trade.fxvps.ai/bridge`). FIX değil
(iki ucu da biz yazıyoruz); protobuf da değil: plugin Windows'un WinHTTP'si
dışında hiçbir kütüphane taşımasın diye. Güncel mesaj tablosu:
[11-mt5-plugin-plan.md](11-mt5-plugin-plan.md) §1 (aşağıdaki tablo ilk taslaktır).

| Yön | Mesaj | İçerik |
|---|---|---|
| P→F | `BridgeHello` | kurum kimliği, sunucu adı, plugin sürümü, protokol sürümü |
| P→F | `BridgeAuth` | kurum API anahtarı (+ mTLS istemci sertifikası) |
| F→P | `BridgeConfig` | yetkili sembol eşlemesi (MT5 adı ↔ bizim ad, basamak, kontrat), gruplar için yürütme profili |
| F→P | `QuoteBatch` | tam hızlı bid/ask (kurum markup'ı uygulanmış) |
| P→F | `BridgeOrder` | MT5 istek kimliği, login, grup, sembol, yön, hacim, tip (market/limit/stop/kapanış), istenen fiyat, sapma (deviation), kapanan pozisyon kimliği |
| F→P | `BridgeFill` | istek kimliği, dolan hacim, ortalama fiyat, LP fiyatı, komisyon, kalan; birden çok parça olabilir |
| F→P | `BridgeReject` | istek kimliği, sebep kodu (fiyat kaydı, likidite yok, kota, oturum yok) |
| P→F | `BridgeReconcile` | sembol başına grupların net pozisyonu (MT5'ten) |
| F→P | `BridgeReconcileResult` | bizim kurum hesabı netiyle fark |
| ↔ | `Heartbeat` | 1 sn; 3 sn sessizlik = kopuk |

Kurallar:
- **Idempotentlik:** anahtar `kurum:mt5_istek_kimliği`. Plugin yeniden
  bağlanınca yanıtsız istekleri aynı kimlikle tekrar gönderir; motor çift
  yürütmez (mevcut client_order_id mekanizması).
- **Kayma ve kısmi dolum:** `BridgeOrder.deviation` motorda müşteri sapması
  olarak kullanılır (#123); grup sınırı ve %0,5 sigorta geçerli. Kısmi dolum
  politikası kurum grubundan.
- **Kopukluk:** plugin bağlantı yokken yeni isteği bekletmez; kurumun
  seçtiği **düşüş modu** uygulanır: reddet (varsayılan) ya da MT5'in kendi
  B-book'una bırak. Açık pozisyonlar etkilenmez.

## 3. Bizim taraftaki yenilikler

1. **Kurum modeli:** `Kurum { id, ad, api_anahtarı_özeti, ip_listesi,
   kurum_hesabı, markup_profili, kota (emir/sn, açık lot), düşüş_modu }`.
   Kurum hesabı mevcut hesap tipidir; teminat ön yatırımla tutulur, risk
   motoru marjı bu hesaba uygular (kurumun müşterilerinin değil).
2. **Köprü kapısı**: client-gateway içinde `/bridge` (ayrı servis gerekmedi).
3. **Mutabakat:** gece nöbetine kurum katmanı: MT5'in net pozisyonu
   (`BridgeReconcile`) ↔ kurum hesabı ↔ LMAX omnibus.
4. **Konsol → Kurumlar:** kurum açma, anahtar üretme, sembol eşleme,
   markup/kota, canlı bağlantı durumu, akış ve gelir raporu.
5. **Sahte MT5 (`tools/mt5-sim`):** köprü protokolünü konuşan Rust istemcisi;
   CI'da uçtan uca: bağlan → fiyat al → 100 login'den emir → dolum →
   yeniden bağlanma ve çift gönderim → mutabakat.

## 4. Plugin (SDK geldiğinde)

- C++17, MT5 Server API; protobuf + WebSocket istemcisi (statik bağlı).
- Sunucu yöneticisi için yapılandırma: kurum anahtarı, uç adresleri (iki
  adres, mavi/yeşil), yetkili gruplar listesi, düşüş modu.
- MT5 tarafında istek yakalama, onay ve tick besleme arayüzlerinin tam adları
  ve iş parçacığı kuralları SDK belgesiyle teyit edilecek; plugin hiçbir
  çağrıda ağ beklemeyecek (kuyruk + ayrı iş parçacığı).
- Teslim: imzalı DLL + kurulum kılavuzu + test MT5 sunucusunda kabul
  sınavı (sahte MT5 ile aynı senaryo listesi).

## 5. Etaplar

| Etap | İş | Bağımlılık |
|---|---|---|
| P1 | `fxvps_bridge_v1.proto`, kurum modeli, bridge-gateway iskeleti | yok |
| P2 | Sahte MT5 + CI uçtan uca | P1 |
| P3 | Konsol "Kurumlar" + kurum mutabakatı | P1 |
| P4 | Plugin (C++) + test MT5 sunucusunda kabul | SDK + test sunucusu |
| P5 | İlk kurum canlıya, gözetimli | P4 |
