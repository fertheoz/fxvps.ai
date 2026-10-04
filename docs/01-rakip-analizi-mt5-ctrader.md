# 01 — Rakip Analizi: MetaTrader 5 ve cTrader

> **Proje:** fxvps.ai. FIX API ile likidite sağlayıcılarına (LMAX vb.) bağlanan, son kullanıcıya alt hesap (sub-account) ve kendi işlem terminalini veren modern platform.
> **Tarih:** 2026-10-04. **Durum:** araştırma, doğrulama turu 2 (genişletilmiş).
> **Hedef okur:** ürün, terminal (frontend), OMS/risk (backend) ve back-office ekipleri.

## Kaynak işaretleri (önce okuyun)

Bu turda `metatrader5.com` ve `help.ctrader.com` sayfaları WebFetch ile **açılamadı**, çünkü ortamın çıkış vekili (egress proxy) bu alan adlarını engelledi (`EGRESS_BLOCKED`). Bu yüzden "resmi dokümanı okudum" anlamına gelen bir işaret **kullanılmıyor**. İşaretler şöyle:

| İşaret | Anlamı |
|---|---|
| **[R]** | Bilgi, **resmi alan adındaki** bir sayfanın (metatrader5.com, mql5.com, help.ctrader.com, spotware.com, ctrader.com) arama sonucu özetinden geliyor. URL gerçek ve aramada döndü, ama sayfa gövdesi tarafımızdan doğrudan okunmadı. Güven: yüksek-orta. |
| **[3P]** | Üçüncü parti kaynak (broker yardım sayfası, bağımsız doküman, haber sitesi). Güven: orta. |
| **[?]** | Belirsiz, çelişkili ya da yalnız genel bilgiye dayalı. Kullanmadan önce doğrulanmalı. |
| *(işaretsiz)* | fxvps.ai için **bizim önerimiz / yorumumuz** (olgu iddiası değil). |

Kaynaklar listesindeki (§20) tüm URL'ler bu turdaki aramalarda gerçekten döndü. Tahminle yazılmış URL kalmadı.

---

## İçindekiler

1. Yönetici özeti
2. Ürün ailesi ve dağıtım yüzeyleri
3. Terminal arayüzü: MT5 masaüstü
4. Terminal arayüzü: MT5 web ve mobil
5. Terminal arayüzü: cTrader (masaüstü, web, mobil)
6. Emir türleri, yürütme modları, doldurma politikaları
7. Emir bileti (order ticket) alanları
8. Pozisyon yönetimi ve hesap modelleri (hedging/netting)
9. Hızlı işlem: one-click, QuickTrade, DoM varyantları
10. Grafik özellikleri
11. Uyarılar, ekonomik takvim, bildirimler
12. Klavye kısayolları
13. Otomasyon ve API'ler
14. Copy trading
15. Sunucu ve back-office
16. Likidite tarafı: FIX ve LMAX
17. UX dersleri
18. Özellik matrisi: v1 / v2 / sonra
19. Açık sorular ve doğrulama listesi
20. Kaynaklar

---

## 1. Yönetici özeti

**MetaTrader 5 (MetaQuotes)**
- Çok varlıklı bir platform: FX, CFD, hisse, vadeli. Hem netting hem hedging hesap modeli var [R].
- Rakamlar: 21 zaman dilimi, 38 yerleşik gösterge, 44 analitik nesne [R].
- Otomasyon: MQL5 dili, MetaEditor, Strategy Tester. Ekosistem: MQL5 Market, Signals, VPS.
- Sunucu tarafı endüstri standardı: Main/History/Access/Backup sunucuları, Administrator, Manager, Gateways, Manager/Web API [R][3P].
- Zayıf yönler: arayüz eski ve kalabalık. Trailing stop **istemci tarafında** çalışır, yani terminal kapanınca iptal olur [R]. Web terminali masaüstünden geri kalır. Ayarlar yerelde tutulur.

**cTrader (Spotware)**
- Modern ve daha sade bir arayüz.
- Yerleşik **bulut Workspaces**: grafikler, QuickTrade, tema, düzen, dil gibi ayarlar cihazlar ve brokerlar arasında taşınır [R].
- 3 DoM tipi (Standard / Price / VWAP) [R/3P]. QuickTrade için tek tık, çift tık ve kapalı modları [3P].
- Trailing stop **sunucu tarafında** çalışır [R: topluluk + Spotware haberleri].
- SL tetikleme yöntemleri (Advanced Protection) [R].
- Otomasyon: C#/.NET 6 cBot'lar, backtest, optimizasyon ve **bulutta cBot çalıştırma** [R].
- Sunucu tarafında çalışan cTrader Copy (performans, yönetim ve hacim ücreti) [R].
- Son kullanıcıya ücretsiz FIX API (QUOTE + TRADE oturumları) ve Open API (Protobuf/JSON, OAuth2) [R].
- Broker tarafı: cBroker back-office [R].

**fxvps.ai için ana sonuç**
- cTrader'ın şeffaflık ve bulut-tutarlılık modelini temel almalıyız: web = masaüstü, sunucu tarafı koruma emirleri, VWAP önizleme.
- Üstüne MT5'in denetlenebilir **order → deal → position** veri modelini ve broker tarafındaki esnek routing/grup yapısını eklemeliyiz.
- Farklılaştırıcı başlıklar: her zaman görünen risk/marj, emir öncesi tam maliyet önizlemesi, onaylı AI asistan, gecikme göstergesi, alt hesap yönetimi.

---

## 2. Ürün ailesi ve dağıtım yüzeyleri

| Yüzey | MetaTrader 5 | cTrader | Not |
|---|---|---|---|
| Masaüstü | Windows yerel; macOS/Linux için paketli sürümler [?] | Windows; ayrıca ayrı **cTrader Mac** uygulaması (help.ctrader.com/ctrader-mac/ yolu var) [R] | |
| Web | MT5 Web Platform. Resmi sürüm haberine göre: netting+hedging, 31 gösterge, 23 analitik nesne, one-click, tüm emir türleri, DoM [R] | cTrader Web. Masaüstüne çok yakın: settings, ASP, chart modes, hotkeys sayfaları ayrı belgelenmiş [R] | MT5 web, masaüstüne göre gösterge/nesne sayısında açıkça geride |
| Mobil | iOS/Android: kotasyon, grafik, işlem, geçmiş, sinyal aboneliği [R] | iOS/Android: grafik, symbol overview, işlem [R] | |
| Kimlik | İşlem hesabı no + parola; MQL5.community hesabı ayrı (sinyal, market) [R] | cTrader ID: workspaces bulut ayarları buna bağlı [R] | |
| Programlama | MQL5, MetaEditor; Python paketi [?] | C# / .NET 6 (cBot, indikatör) [R] | |
| İstemci API | Resmi son kullanıcı API'si yok [?] | Open API (Protobuf port 5035 / JSON port 5036), FIX API [R][3P] | |
| Broker ürünleri | Server, Administrator, Manager, Gateways, Manager API (C++/.NET), Web API [R][3P] | cBroker (back office), Spotware barındırmalı [R] | |

---

## 3. Terminal arayüzü: MT5 masaüstü

### 3.1 Genel yerleşim
Varsayılan pencerede beş ana alan var:

```
┌──────────────────────── Menü + araç çubukları ────────────────────────┐
│ Market Watch │                                                        │
│ (Ctrl+M)     │            Grafik pencereleri (MDI, sekmeli)           │
├──────────────┤        + One Click Trading paneli (sol üst köşe)       │
│ Navigator    │                                                        │
│ (Ctrl+N)     │                                                        │
├──────────────┴────────────────────────────────────────────────────────┤
│ Toolbox (Ctrl+T): Trade | Exposure | History | News | Mailbox |       │
│                  Calendar | Alerts | ... | Journal                    │
└───────────────────────────────────────────────────────────────────────┘
```

### 3.2 Market Watch
- Sembol listesi (Bid/Ask, spread, günlük değişim gibi sütunlar). Ctrl+M ile açılıp kapanır [R].
- Sağ tık menüsünden New Order, Chart Window, **Depth of Market**, Specification, Hide, Symbols [3P].
- Alt sekmeler: Symbols / Details / Trading / Ticks [?] (genel bilgi, bu turda doğrulanmadı).
- **Specification penceresi** (sembol özellikleri): kontrat büyüklüğü, digits, stop level, marj ve swap parametreleri, seanslar [?].

### 3.3 Navigator
- Ctrl+N [R]. Ağaçta Accounts, Indicators, Expert Advisors, Scripts, Services, Market, Signals, VPS.
- Accounts dışındaki her öğeye kullanıcı **özel kısayol** atayabilir (bağlam menüsünde "Set hotkey"). Atanan kısayol yerleşik olanlardan önceliklidir [R].

### 3.4 Toolbox sekmeleri
Ctrl+T ile açılır (macOS'ta Command+T) [3P].

| Sekme | İçerik | Kaynak |
|---|---|---|
| Trade | Bakiye, Equity, Margin, Free Margin, Margin Level; açık pozisyonlar ve bekleyen emirler | [3P] |
| Exposure | Açık pozisyonların döviz bazında hacim özeti | [3P] |
| History | Kapanmış pozisyonlar, iptal edilen emirler, deals | [3P] |
| News | Piyasa haberleri | [3P] |
| Mailbox | Brokerdan iç posta | [3P] |
| Calendar | Ekonomik takvim | [3P][R] |
| Alerts | Kurulu uyarılar | [3P] |
| Journal | Terminal olayları ve işlemler günlüğü | [3P][R] |
| Experts | EA günlükleri | [?] |
| Market / Signals / Code Base / Articles / VPS | MQL5 ekosistem sekmeleri | [?] |

**Gözlem:** sekme sayısı 10'u aşıyor. İşlemle ilgili olanlar (Trade/History/Journal) ile pazar ve içerik sekmeleri (Market/Articles/Code Base) karışık duruyor. Bu, yeni kullanıcıyı yoruyor (bizim değerlendirmemiz).

### 3.5 Grafik pencereleri
- MDI yapısında: sekmeli, tile/cascade düzenleri. Grafikler ana pencereden ayrılıp (undock) ikinci monitöre taşınabilir [?].
- Chart ayarlarındaki "Show quick trading buttons" seçeneği one-click panelini ve DoM düğmesini grafikte gösterir ya da gizler [R].
- Profiller (grafik düzeni kümesi) ve şablonlar (.tpl) yerelde saklanır [?].

### 3.6 Depth of Market (MT5)
- Market Watch sağ tık → "Depth of Market" ya da **Alt+B** ile açılır [3P: BlackBull]. Resmi hotkey listesindeki Ctrl+B "Objects List" içindir. Alt+B'nin resmi listede olup olmadığı **[?]**.
- İki tür işlem yapılır [3P]:
  - alttaki emir alanından **market** alım/satım,
  - tablodaki fiyat seviyesine tıklayarak **pending** emir.
- One-Click Trading açıksa talep onay iletişim kutusu çıkmadan doğrudan sunucuya gider [R].
- DoM üzerinden açık pozisyon ve bekleyen emirlerin SL/TP seviyeleri tek tıkla yönetilebilir (web sürüm haberi) [R].

### 3.7 Data Window, Strategy Tester ve diğer pencereler
- Data Window: imleç altındaki mumun OHLC değerleri ve gösterge değerleri [?].
- Strategy Tester: ayrı panel. Ctrl+R [?].
- Task Manager: F2 [R].
- F7: önceki grafik penceresine geçiş [R: hotkey özeti].

---

## 4. Terminal arayüzü: MT5 web ve mobil

### 4.1 MT5 Web Platform [R]
- Resmi sürüm duyurusundaki özellikler:
  - netting ve hedging,
  - **31 gösterge**, **23 analitik nesne** (masaüstünde 38 ve 44),
  - one-click trading,
  - "tüm emir türleri",
  - **Depth of Market**: beta sürümde yoktu, resmi sürümle geldi.
- Herhangi bir tarayıcı ve işletim sisteminden FX ve borsa işlemi yapılabilir.
- Bulunmayanlar (genel bilgi **[?]**): EA ve script çalıştırma, özel gösterge, Strategy Tester.
- Broker, web terminalini kendi sitesine gömebilir [?].

### 4.2 MT5 mobil (iOS/Android) [R]
- Mobil yardımın kendi ağacı var: Trade → Trading Principles → Types of Orders, Fill Policy, Opening/Closing Positions, Placing Pending Orders.
- **Instant Execution** gibi yürütme modlarına göre ayrı ekran akışları var.
- Sinyallere mobilden abone olunabiliyor (MetaQuotes haberi).
- Push bildirimleri: MetaQuotes ID ile uyarılar ve işlem bildirimleri [R: alerts sayfası özeti].

---

## 5. Terminal arayüzü: cTrader (masaüstü, web, mobil)

### 5.1 Genel yerleşim

```
┌ app bar ┬──────────── Üst çubuk: arama, QuickTrade Mode, hesap ────────────┐
│ Trade   │ Market Watch / │                                  │ Active Symbol │
│ Copy    │ Watchlists     │   Grafik alanı                   │ Panel (ASP)   │
│ Algo    │ (QuickTrade    │   (Multi / Single / Free mod)    │ - emir        │
│ Analyze │  butonları)    │                                  │ - sentiment   │
│         │                │                                  │ - market hrs  │
│         ├────────────────┴──────────────────────────────────┤ - DoM         │
│         │ TradeWatch (Ctrl+W): Positions | Orders | History │               │
│         │ | Transactions | Journal ...                      │               │
└─────────┴───────────────────────────────────────────────────┴───────────────┘
```
Sol kenardaki uygulama çubuğunun içeriği sürüme göre değişir **[?]**. TradeWatch sekme adları bu turda resmi kaynakla doğrulanamadı **[?]**.

### 5.2 Market Watch / Watchlists [R]
- Sembolleri içerir. QuickTrade moduyla doğrudan işlem açılabilir. Semboller izleme listelerinde gruplanır.
- Watchlist'te seçilen sembol, sağdaki ASP'nin aktif sembolü olur.
- Ayarlar [3P: FxPro / help settings özeti]:
  - "Daily change" ve "Single click in Market Watch" açılıp kapatılabilir,
  - **çift tıklama eylemi** seçilebilir: New order penceresi, yeni grafik, yeni ayrık (detached) grafik ya da kapalı.

### 5.3 Active Symbol Panel (ASP) [R]
- Seçili sembolün tüm ayrıntıları ve doğrudan emir girişi.
- Bölümler: **Market Sentiment**, Market Details, Trade Statistics, **Market Hours**, Inverted Rate, Leverage, **Depth of Market**.
- **Market Sentiment**: sembolde açık pozisyonu olan cTrader hesaplarından yükseliş ve düşüş bekleyenlerin yüzdesi.
- **Market Hours**: haftalık açılış takvimi. Bugünün tarihi yeşil noktayla işaretli. Kapanışa veya açılışa kalan süre gerçek zamanlı gösterilir.
- Hızlı kontroller: yeni grafik, yeni emir, sembol değiştirme.
- Ctrl+E ile gösterilip gizlenir [R].

### 5.4 Grafik modları ve detach [R]
- Üç mod:
  - **Multi Chart** (varsayılan): grafik eklenir, sürüklenip yerleştirilir.
  - **Single Chart**: tek grafik, diğerleri üstte sekme olarak durur.
  - **Free Chart**: Multi gibi ama her grafik ayrı ayrı boyutlandırılır.
- **Detach Chart**: grafik kendi araç çubuğuyla yeni pencerede açılır. "Reattach" ile geri döner.
- F2 grafik modunu, F3 düzeni değiştirir [R].

### 5.5 Workspaces (bulut) [R]
- Workspaces, cTrader'ın bulut özelliğidir. Ayarlar **birden çok hesap, farklı brokerlar ve farklı cihazlar** arasında kullanılır.
- Saklanan ayarlar: grafikler ve grafik yapılandırması, Quick Trade parametreleri, renk teması, düzen, dil, ses, varlık gösterim ayarları, sembol listesi ayarları, uygulama içi bildirim ayarları.
- F7/F8 ile önceki ve sonraki workspace'e geçilir [R].

### 5.6 cTrader Web ve Mac
- help.ctrader.com'da `ctrader-web/` ve `ctrader-mac/` için ayrı ağaçlar var: settings, active-symbol-panel, chart-modes, hotkeys, orders, protections [R].
- Bu, Spotware'in web ve masaüstünü **aynı özellik setiyle** sürdürdüğünü gösteriyor. "Web 3.0 All-in-One Experience" duyurusu da bunu destekliyor [R].

### 5.7 cTrader mobil
- Symbol overview, charts (Tick/Renko/Range dahil grafik türleri) sayfaları var [R].

### 5.8 UI bileşeni karşılaştırması

| Bileşen | MT5 | cTrader | fxvps.ai kararı |
|---|---|---|---|
| Sembol listesi | Market Watch | Market Watch + Watchlists + sentiment | Çoklu liste; **gerçek maliyet** sütunu (spread + komisyon) |
| Sembol ayrıntısı | Specification modal penceresi | ASP (sürekli açık sağ panel) | Sağ panel (ASP tarzı) + satır içi kart |
| Pazar saatleri | Specification içinde seans tablosu [?] | ASP → Market Hours, geri sayımlı | Geri sayım + tatil uyarısı (v1) |
| Alt panel | Toolbox (10+ sekme) | TradeWatch | En fazla 5-6 sekme: Pozisyonlar, Emirler, Geçmiş, Hareketler, Günlük, Uyarılar |
| Grafik düzeni | MDI tile/cascade | Multi / Single / Free | Izgara (1/2/4/6) + serbest |
| Ayrık pencere | Undock [?] | Detach/Reattach | Tarayıcı pop-out (v2), Tauri çoklu pencere |
| Ayar senkronu | Yerel profil | Bulut Workspaces | **Bulut workspace (v1)** |
| Sentiment | Yok [?] | Var | v2 (alt hesap verisi yeterli büyüklüğe ulaşınca) |
| Takvim | Toolbox → Calendar | [?] | v2 |

---

## 6. Emir türleri, yürütme modları, doldurma politikaları

### 6.1 MT5 emir türleri [R]
- **Market**: Buy, Sell.
- **Pending**:
  - Buy Limit: Ask belirtilen fiyata eşit ya da altındayken alım.
  - Sell Limit: Bid belirtilen fiyata eşit ya da üstündeyken satış.
  - Buy Stop: Ask belirtilen fiyata eşit ya da üstündeyken alım.
  - Sell Stop: Bid belirtilen fiyata eşit ya da altındayken satış [R: Buy Limit/Buy Stop/Sell Limit tanımları; Sell Stop simetrik].
- **Stop Limit**: Buy Stop Limit ve Sell Stop Limit. Örnek: Buy Stop Limit'te Ask stop seviyesine (piyasanın üstünde) ulaşınca, o seviyenin altındaki Stop Limit fiyatına bir Buy Limit emri yerleştirilir [R].
- **Stop Loss / Take Profit**: pozisyona ve bekleyen emre bağlanır [R].
- **Trailing Stop**: terminalde çalışır, sunucuda değil. Platform kapanınca trailing iptal olur. Pending emre de atanabilir: emir dolunca aynı yöndeki pozisyona uygulanır [R: release notes 261 + yardım özeti].
- **OCO**: yerleşik değil, EA ile yapılır **[?]**.

### 6.2 MT5 yürütme modları ve doldurma politikaları [R]
Yürütme modu brokerın sembol ayarıdır. Her mod hangi doldurma politikalarına izin verildiğini belirler:

| Yürütme modu | Anlamı (özet) | İzinli doldurma politikaları |
|---|---|---|
| Instant Execution | Gösterilen fiyattan; sapma aşılırsa requote | Fill or Kill |
| Request Execution | Önce fiyat istenir, sonra onay [?] | (FOK [?]) |
| Market Execution | Broker/LP fiyatından; requote yok | FOK, IOC, Return |
| Exchange Execution | Borsa emir defteri | FOK, IOC, **BOC**, Return |

- **Fill or Kill**: emir yalnız tam hacimle dolar. Hacim birden fazla teklifle tamamlanabilir.
- **Immediate or Cancel**: mümkün olan en fazla hacim dolar, kalanı iptal edilir. Kullanılıp kullanılamayacağını sunucu belirler.
- **Book or Cancel**: emir yalnızca deftere yazılabilir. Hemen dolacaksa iptal edilir (yalnız exchange modunda).
- **Return**: kısmi dolumda kalan hacim iptal edilmez, işlenmeye devam eder. Market emirlerinde yalnız Exchange modunda; limit ve stop-limit'te Market ve Exchange modlarında [R].

### 6.3 MT5 emir süresi (expiration) [R]
- GTC: elle iptal edilene kadar.
- Today: o işlem günü.
- Date and Time: belirtilen ana kadar.
- (Specified Day seçeneği var **[?]**.)

### 6.4 cTrader emir türleri [R][3P]
- **Market** ve **Market Range**: Market Range, kabul edilen slipaj aralığını sınırlar **[?]**. Ayrıntı help.ctrader.com/ctrader/trading/orders sayfasında, bu turda gövdesi okunamadı.
- **Limit**, **Stop**, **Stop Limit**. FIX API kanalı market, stop ve limit emirlerini destekler [R]. Topluluk forumunda FIX üzerinden stop-limit uygulaması tartışılmış **[?]**.
- **Trailing Stop (sunucu tarafı)** [R: Spotware haberleri + topluluk]:
  - cTrader kapalıyken de çalışır.
  - Fiyat lehte hareket ettikçe her 1 pipte güncellenir, aleyhte hareket ettiğinde yerinde kalır.
  - Kullanıcı yalnız "kaç pip geriden izleyeceğini" girer.
- **Advanced Stop Loss**: trailing ve **move to break-even** (topluluk duyurusu başlığı) [R].
- **Stop tetikleme yöntemleri (StopTriggerMethod)** [R]:

| Yöntem | Davranış |
|---|---|
| Trade Side (varsayılan) | Pozisyonun işlem tarafı fiyatı tetikler |
| Opposite Side | Karşı fiyat akışı tetikler |
| Double Trade Side | İşlem tarafı fiyatı **art arda 2 tikte** seviyeyi geçmeli. Buy pozisyonda iki ardışık Bid ≤ seviye |
| Double Opposite Side | Karşı akış iki kez geçmeli (en sıkı) |

- Bu ayar cTrader Automate API'sinde `StopTriggerMethod` olarak açık [R].

### 6.5 Emir türü karşılaştırması

| Özellik | MT5 | cTrader | fxvps.ai |
|---|---|---|---|
| Market | Var | Var | v1 |
| Slipaj sınırı | Deviation (Instant modda) [R] | Market Range [R/?] | v1: "max slipaj" alanı → OMS'te limit-IOC'ye çevrilir |
| Limit / Stop | Var | Var | v1 |
| Stop-Limit | Var [R] | Var [?] | v1 |
| Trailing | **İstemci tarafı** [R] | **Sunucu tarafı** [R] | **v1 sunucu tarafı** |
| Break-even'e taşı | EA ile [?] | Advanced SL [R] | v1 (tek tık + otomatik koşullu) |
| SL tetikleme yöntemi | Sabit (Bid/Ask) [?] | 4 yöntem [R] | v2 (tek ve çift tik) |
| OCO / bracket | Yok (EA) [?] | Yok (cBot) [?] | v1 bracket, v2 OCO |
| Iceberg | Yok [?] | Yok [?] | Sonra (LP destekliyorsa geçir) |
| Doldurma politikası | FOK/IOC/BOC/Return [R] | [?] | FIX TimeInForce ile birebir eşleme |
| Süre | GTC/Today/Date [R] | GTC/tarih [?] | v1: GTC, Day, GTD |

---

## 7. Emir bileti (order ticket) alanları

### 7.1 MT5 "New Order" penceresi (F9) [R]

| Alan | Açıklama |
|---|---|
| Symbol | Enstrüman |
| Type | Yürütme modu seçilirse market işlem; değilse pending emir tipi |
| Volume | Lot |
| Stop Loss | Fiyat **ya da** puan mesafesi (platform ayarına bağlı) |
| Take Profit | Aynı şekilde |
| Comment | İsteğe bağlı, **en fazla 31 karakter** |
| Deviation | Kabul edilen fiyat sapması (Instant/Request). Büyükse requote olasılığı düşer |
| Expiration | GTC / Today / Date and Time (pending) |
| (Price, Stop Limit price) | Pending için fiyat; stop-limit için ikinci fiyat [R: emir türü tanımı] |
| Fill policy | Sembolün yürütme moduna göre seçilir [R] |

### 7.2 cTrader "Create Order" penceresi [3P/?]
- Sekmeler: Market / Limit / Stop / Stop Limit [?].
- Alanlar:
  - Hacim (lot ya da birim; ön ayar düğmeleri),
  - Market Range,
  - Stop Loss ve Take Profit (pip ya da fiyat),
  - **Trailing Stop** onay kutusu,
  - SL tetikleme yöntemi,
  - Expiry,
  - Comment/Label [?].
- QuickTrade kapalıyken Bid/Ask düğmesine tıklamak bu pencereyi açar [3P].
- Ticket'ta pip değeri ve marj tahmini gösterildiği yaygın olarak bildiriliyor **[?]**.

### 7.3 fxvps.ai emir bileti önerisi (v1)

| Alan | Davranış |
|---|---|
| Yön | Buy/Sell büyük düğmeler; canlı Bid/Ask ve spread |
| Tür | Market · Limit · Stop · Stop-Limit (sekme) |
| Hacim | Lot ↔ birim ↔ **risk %** ↔ **risk tutarı** çift yönlü hesap |
| Fiyat(lar) | Limit/Stop fiyatı; stop-limit'te ikinci fiyat; mesafe pip olarak eş zamanlı |
| Max slipaj | Pip; boşsa saf market |
| SL / TP | Fiyat, pip, para birimi ya da % (4 giriş modu, biri değişince diğerleri güncellenir) |
| Trailing | Pip mesafesi + adım; sunucu tarafı |
| Break-even | "X pip kârda SL'yi girişe taşı" |
| Süre | GTC / Day / GTD |
| Etiket/Not | 64 karakter |
| **Önizleme** | VWAP doldurma tahmini (LP derinliğinden), gerekli marj, işlem sonrası marj seviyesi, spread + komisyon maliyeti, gecelik swap |
| Onay | Ayara göre: tek tık / çift tık / onay penceresi (cTrader modeli) |

---

## 8. Pozisyon yönetimi ve hesap modelleri

### 8.1 MT5 hesap modeli [R]
- **Netting**:
  - sembol başına tek ortak pozisyon,
  - aynı yönde işlem hacmi artırır, ters yönde işlem azaltır, kapatır ya da ters çevirir,
  - kapatmak için aynı hacimde ters işlem yapılır.
- **Hedging**:
  - aynı sembolde birden çok pozisyon, karşıt pozisyonlar dahil,
  - her yeni deal yeni bir pozisyon açar,
  - kapatmak için bağlam menüsünden "Close Position" seçilir.
- Hangi modelin kullanılacağına broker hesap bazında karar verir.
- Varlık modeli: **order** (talep) → **deal** (gerçekleşen işlem) → **position** (açık durum) [R: genel kavramlar sayfası].
- **Close By** (karşıt pozisyonla kapatma): yalnız hedging'de **[?]**.
- Kısmi kapatma: hacim girilerek yapılır [?].

### 8.2 cTrader hesap modeli [R]
- Tek ortamda hem **hedged** hem **netted** hesaplar. Spotware bunu ilk yapan platform olduğunu söylüyor.
- Ortak ayarlar ve ortak broker raporlama araçları. Arayüz değişmiyor. Netting, cAlgo, web ve mobilde de geçerli.
- **FIFO Netting**: ABD NFA kurallarına uygun (RFED'ler için).
- Pozisyon eylemleri: kısmi kapatma, toplu kapatma (tümü / kârlılar / zarardakiler / alışlar / satışlar), **Reverse**, **Double Up** **[?]**. QuickTrade ile "hacim ekle / pozisyonu ters çevir" tek tıkla yapılabiliyor [3P].

### 8.3 Marj ve stop-out
- **Margin Level = Equity / Margin × 100** [3P: Pepperstone].
- Tipik broker değerleri: Margin Call %90, Stop Out %50. Bunlar platform sabiti değil, **grup ayarıdır** [3P].
- Stop-out sırasında kapatma genellikle **en büyük zarardaki pozisyondan** başlar [3P].
- cBroker 9.5: çok yüksek kaldıraç ayarları, **toplu zorla kapatma** (bulk force-close), **marj bypass** seçenekleri [R].

### 8.4 fxvps.ai karar önerisi
- v1: yalnız hedging. Veri modeli MT5 gibi order/deal/position ayrımıyla kurulur.
- v2: netting ve FIFO bayrağı (ABD veya belirli regülasyonlar için).
- Stop-out motoru sunucuda, tik bazlı çalışır. Kapatma sırası yapılandırılabilir (en büyük zarar / en büyük marj). Kullanıcı arayüzünde "stop-out'a kalan mesafe" sürekli görünür.

---

## 9. Hızlı işlem: one-click, QuickTrade, DoM varyantları

### 9.1 MT5 One-Click Trading [R]
- Grafiğin sol üst köşesinde bir panel. Açıldığında talepler onay penceresi olmadan sunucuya gider.
- İlk açılışta kullanıcı koşulları kabul eder [?].
- DoM'dan yapılan işlemler de bu ayara tabidir.

### 9.2 cTrader QuickTrade [3P/R]
| Mod | Davranış |
|---|---|
| Single-Click | Bid (SELL) ya da Ask (BUY) düğmesine bir tık → emir gider |
| Double-Click | Çift tık → emir gider; tek tık bir şey yapmaz |
| Disabled | Tıklama "Create Order" penceresini açar (iki adım) |

- Mod, sağ üstteki **QuickTrade Mode** düğmesiyle ya da Settings → Quick Trade bölümünden değiştirilir.
- Mobilde yanlışlıkla tetiklenme şikayetleri var (topluluk) [R]. Bu bir UX dersi: mobilde varsayılan **Double** ya da **Disabled** olmalı.

### 9.3 cTrader DoM varyantları [R][3P]
- **Standard DoM**: fiyat seviyelerinde alım ve satım hacimleri listesi, klasik emir defteri.
- **Price DoM**: dikey fiyat merdiveni. Tıklanan seviyeye limit veya stop emir yerleştirilir.
- **VWAP DoM**: seçilen hacim için hacim ağırlıklı ortalama doldurma fiyatını gösterir. "Bu büyüklükte market emri hangi ortalama fiyattan dolar?" sorusuna yanıt verir.
- Bu ayrıntılar Finance Magnates haberine ve help sayfası başlığına dayanıyor [3P/R]. Her varyantın bütün davranışları **[?]**.

### 9.4 Karşılaştırma

| | MT5 | cTrader | fxvps.ai |
|---|---|---|---|
| Tek tık | Panel (aç/kapa) | 3 mod | 3 mod; mobil varsayılanı Double |
| DoM sayısı | 1 | 3 | v1: Standard + Price + **VWAP önizleme** |
| DoM'dan SL/TP | Var (web haberi) [R] | Var [?] | v1 |
| Derinlik kaynağı | Broker feed / borsa L2 | Broker LP agregasyonu | LMAX PriceDepth (FIX) [3P/?] |

---

## 10. Grafik özellikleri

### 10.1 MT5 [R]
- **21 zaman dilimi**, M1'den MN1'e. Ara değerler: M2, M3, M4, M5, M6, M10, M12, M15, M20, M30, H1, H2, H3, H4, H6, H8, H12, D1, W1, MN1. Liste genel bilgiye dayanıyor, toplam sayı [R].
- **38 gösterge**. Gruplar: trend, osilatör, hacim, Bill Williams.
- **44 analitik nesne**: Fibonacci, Gann, kanallar ve diğerleri.
- Grafik tipleri: Bar, Candle, Line [?].
- Takvim olaylarının grafikte gösterimi [?].

### 10.2 cTrader [R][3P]
- Grafik türleri:
  - **Standard** (zaman bazlı),
  - **Tick** (N fiyat değişiminde bir bar),
  - **Renko** (pip cinsinden sabit tuğla, zamandan bağımsız),
  - **Range** (N pip harekette bir bar),
  - **Heikin-Ashi**.
- Bar tipleri hotkey'lerle değişir: Alt+1 Bar, Alt+2 Candlestick, Alt+3 Line, Alt+4 Dot [R].
- Çizimler:
  - Ctrl+Click ile seçime eklenir, Ctrl+Drag ile kopyalanır, Alt+Drag ile yapışmadan taşınır,
  - Shift ile 45° çizgi, kare ya da daire çizilir [R].
- Zaman dilimi sayısı ve yerleşik gösterge sayısı bu turda **doğrulanamadı [?]**. Topluluk özel zaman dilimi istiyor. Store'da "Custom Timeframes" ürünleri var, bu da yerleşik desteğin sınırlı olduğunu düşündürüyor [R].
- Ctrl+S ile "market snapshot" (geçmiş bir noktadaki fiyat ve zaman bilgisini okuma), Ctrl+B ve Ctrl+K ile Bid/Ask çizgisi [R].

### 10.3 Karşılaştırma

| | MT5 | cTrader | fxvps.ai v1 |
|---|---|---|---|
| Zaman dilimi | 21 | [?] + tick/renko/range | M1, M5, M15, M30, H1, H4, D1, W1, MN + özel (dakika cinsinden) |
| Alternatif grafik | Yok [?] | Tick, Renko, Range, HA | HA v1; Tick, Renko, Range v2 |
| Gösterge | 38 (web 31) | [?] | Kütüphaneye bağlı (bkz. 10.4) |
| Çizim | 44 (web 23) | [?] | Kütüphaneye bağlı |
| Grafikten işlem | Var | Var | Var (sürükle SL/TP, emir çizgisi) |
| Çoklu grafik | MDI | 3 mod | Izgara + serbest |

### 10.4 Grafik kütüphanesi seçimi [3P]
- **TradingView Lightweight Charts**:
  - Apache-2.0 lisanslı, ücretsiz.
  - TradingView atfı zorunlu (`attributionLogo` seçeneği ya da kullanıcıya görünen sayfada tradingview.com bağlantısı).
  - Gösterge ve çizim araçları yerleşik değil; kendimiz yazarız.
- **TradingView Advanced Charts / Trading Platform**:
  - Onaylanan şirketlere atıf karşılığında ücretsiz.
  - Yalnız **halka açık** web projeleri için. Paywall arkası ve kişisel kullanım kapsam dışı.
  - Ücretli sürüm fiyatı yayımlanmamış; üçüncü taraf yıllık ~144 bin USD bildiriyor **[?]**.
- **Sonuç:** fxvps.ai terminali giriş arkasında (paywall benzeri) çalışacağı için Advanced Charts'ın ücretsiz kapsamına girip girmeyeceği **hukuki ve satış teyidi** gerektiriyor. v1'de Lightweight Charts ile kendi gösterge ve çizim katmanımızı yazmak daha güvenli görünüyor.

---

## 11. Uyarılar, ekonomik takvim, bildirimler

### 11.1 MT5 uyarıları [R][3P]
- Koşullar: Bid >/<, Ask >/<, Last >/<, Volume >/<, Time =.
- Eylemler: ses, dosya çalıştırma, e-posta, **push bildirimi** (mobile).
- Parametreler: tekrar aralığı (timeout, saniye), en fazla tekrar sayısı, bitiş zamanı.
- Toolbox → Alerts sekmesinden yönetilir.

### 11.2 MT5 ekonomik takvim [R]
- Build 2005 ile yenilendi: tescilli takvim, **600+ haber ve gösterge**, **13 büyük ekonomi** (ABD, AB, Japonya, İngiltere, Kanada, Avustralya, Çin ve diğerleri). Veri açık kaynaklardan gerçek zamanlı toplanıyor.
- **MQL5'ten programatik erişim**: olayları okuma, filtreleme, değişiklikleri izleme. EA'lar habere tepki verebiliyor.
- Aynı build'de "MQL5 applications as services" geldi.

### 11.3 cTrader
- Fiyat uyarıları ve uygulama içi bildirim ayarları workspace'te saklanıyor [R]. Ayrıntılı koşul seti **[?]**.
- Yerleşik ekonomik takvim ve haber: broker ve sürüme bağlı **[?]**.

### 11.4 fxvps.ai önerisi
- v1 uyarı koşulları: fiyat (Bid, Ask, Mid; geçiş yönüyle), % değişim, **marj seviyesi**, pozisyon P/L, emir doldu/iptal, SL/TP tetiklendi.
- Kanallar: Web Push, e-posta, **Telegram** (mevcut altyapı var), uygulama içi.
- Uyarılar sunucuda değerlendirilir (MT5'teki istemci bağımlılığından kaçınmak için).
- Takvim v2'de: lisanslı veri sağlayıcıyla, grafikte olay pinleri ve "olay öncesi uyar" seçeneği.

---

## 12. Klavye kısayolları

### 12.1 MT5 (resmi "Hot Keys" sayfasından özet) [R]
| Kısayol | Eylem |
|---|---|
| F9 | New Order penceresi |
| Ctrl+M | Market Watch aç/kapa |
| Ctrl+N | Navigator aç/kapa |
| Ctrl+T | Toolbox aç/kapa |
| Ctrl+B | Objects List (nesne listesi) |
| F2 | Task Manager |
| F7 | Önceki grafik penceresi |
| ← / → | Grafiği kaydır |
| ↑ / ↓ | Hızlı kaydırma |
| + / − | Yakınlaştır / uzaklaştır |
| Home / End | Grafiğin başı / sonu |
| Alt+B | Depth of Market [3P, resmi listede olup olmadığı **?**] |
| Ctrl+D, Ctrl+R, Ctrl+G, Ctrl+F | Data Window, Strategy Tester, ızgara, crosshair **[?]** (genel bilgi) |
| (özel) | Navigator öğelerine "Set hotkey". Hesaplar hariç. Yerleşik olanlardan önceliklidir |

### 12.2 cTrader (resmi hotkeys sayfasından özet) [R]
| Kısayol | Eylem |
|---|---|
| Space | Akıllı grafik araması |
| F1 | Yardım |
| F2 | Grafik modunu değiştir |
| F3 | Düzeni değiştir |
| F7 / F8 | Önceki / sonraki workspace |
| F9 | Yeni emir ekranı |
| F11 | Tam ekran |
| Ctrl+Q | Menü göster/gizle |
| Ctrl+W | TradeWatch göster/gizle |
| Ctrl+E | Active Symbol Panel göster/gizle |
| Ctrl+Tab / Ctrl+Shift+Tab | Sonraki / önceki grafik |
| Ctrl+F | Tüm sembollerde ara |
| Ctrl+D | Çizimleri yönet |
| Ctrl+I | Göstergeleri yönet |
| Ctrl+S | Market snapshot aç/kapa |
| Ctrl+G | Izgara |
| Ctrl+B / Ctrl+K | Bid / Ask çizgisi |
| Alt+1..4 | Bar / Mum / Çizgi / Nokta |
| Ctrl+Tekerlek | Yakınlaştır |
| Orta tık | Crosshair |
| Ctrl+Click / Ctrl+Drag / Alt+Drag | Çizim seçimi / çoğaltma / yapışmasız taşıma |
| Shift + çizgi/dikdörtgen/elips | 45° çizgi / kare / daire |
| (özel) | Application settings'te tüm hotkey'ler yeniden atanabilir |

### 12.3 fxvps.ai önerisi
- F9 = yeni emir (iki platformda da aynı; kas hafızası). Space ya da Ctrl+K = komut paleti ve sembol araması.
- Tüm kısayollar yeniden atanabilir, workspace'te saklanır.
- **İşlem kısayolları** (Buy, Sell, Close all) varsayılan olarak **kapalı** gelir. Açılınca bir "silahlı" göstergesi görünür.
- macOS'ta Cmd eşlemesi.

---

## 13. Otomasyon ve API'ler

### 13.1 MQL5 [R][?]
- Program türleri: Expert Advisor, Indicator, Script, **Service** (build 2005'ten beri), Library.
- MetaEditor IDE (kendi kısayol sayfası var [R]).
- Strategy Tester: gerçek tik modu, genetik optimizasyon, MQL5 Cloud Network [?].
- Ekonomik takvim API'si [R].
- Python entegrasyonu: `MetaTrader5` paketi **[?]**. R dili API'si build 2005 notlarında geçiyor [R].

### 13.2 cTrader Automate (cTrader Algo) [R]
- cBot, özelleştirilebilir parametreleri ve metotları olan bir C# sınıfıdır. İndikatörler de aynı şekilde yazılır.
- **.NET 6** desteği var. Eski .NET Framework 4.x algoları ayrı bir alt süreçte çalışır. Derleme .NET araçlarıyla yapılır: herhangi bir IDE (Visual Studio, Rider) ve NuGet paketleri kullanılabilir.
- **Backtesting**: tik ya da bar verisi, ayarlanabilir spread ve komisyon.
- **Optimizasyon**: parametre ızgarası, seçilen ölçüte göre sıralama.
- **Cloud execution**: cBot'lar güvenli bulut ortamında, cihazdan bağımsız 7/24 çalışır, VPS gerekmez.
- AccessRights (sandbox izinleri) **[?]**: bu turda doğrulanmadı.

### 13.3 Dış API'ler

| API | MT5 | cTrader |
|---|---|---|
| Son kullanıcı programatik erişim | Terminal üzerinden (MQL5 / Python) [?] | **Open API**: TCP veya WebSocket; Protobuf (port 5035) veya JSON (port 5036); OAuth2 (clientId/secret + access/refresh token); bölgesel proxy'ler ve AWS Global Accelerator; resmi kütüphaneler: OpenApiPy (Python/Twisted), OpenAPI.Net [R][3P] |
| FIX | Broker/gateway tarafı | **cTrader FIX API**: her cTrader hesabına ücretsiz; kimlik bilgileri Desktop ve Web'den alınır; `TargetSubID` = QUOTE / TRADE; derinlikli spot fiyat, market/stop/limit emir, pozisyon ve emir durumu [R] |
| Broker API | **Manager API** (C++/.NET; hesap açma ve kapama, grup değişimi, bakiye işlemleri, veri çekme) [3P]; **Web API** (REST + WebSocket) [3P]; Server/Plugin API [?] | cBroker API'leri [?] |

### 13.4 fxvps.ai önerisi
- v2: kullanıcı REST + WebSocket API'si. OAuth2 ve kapsamlı (scoped) anahtarlar kullanır. Hız sınırları belgelenir.
- Sonra: son kullanıcıya **FIX 4.4** (QUOTE/TRADE ayrımı cTrader'daki gibi).
- Otomasyon (sonra): TypeScript/Python stratejiler, izole sandbox (WASM ya da tek kullanımlık kap), "bulutta çalıştır" varsayılan. MQL veya C# uyumluluğu hedeflenmez. AI ile strateji taslağı + zorunlu backtest kapısı.

---

## 14. Copy trading

### 14.1 MQL5 Signals [R]
- Abonelik için MQL5.community hesabı gerekir. Aylık ücret alınabilir.
- Seçenekler: SL/TP'yi kopyala, sermayenin yüzde kaçı kullanılsın (**en fazla %95**), sermaye koruma için stop.
- **Abonenin terminali sürekli açık ve bağlı olmalı.** Bu yüzden MetaQuotes VPS kiralanması öneriliyor; sinyal için sanal barındırma en az 15 USD.
- Mobilden abonelik mümkün.
- MetaQuotes'un sağlayıcıdan aldığı komisyon oranı **[?]**.

### 14.2 cTrader Copy [R]
- Kopyalama **sunucu tarafında** yapılır; yatırımcı cihazı gerekmez [?: mimari çıkarım, ücret sayfası doğrudan söylemiyor].
- Ücret türleri (sağlayıcı istediği gibi birleştirir):
  - **Performance fee**: net kâr üzerinden, **high-water mark**, en fazla **%30**.
  - **Management fee**: yıllık % (günlük tahakkuk), en fazla **%10**.
  - **Volume fee**: kopyalanan milyon başına, taraf başına, en fazla **10 USD**.
- **Equity Stop Loss**: strateji başına.
- Tüm ücretler "Start copying" düğmesinde önceden gösterilir.
- Bir broker (VARIANSE) cTrader müşterilerinin çeyreğinin copy kullandığını bildirmiş [3P].

### 14.3 fxvps.ai önerisi (v2)
- Alt hesap mimarisi copy için doğal: "copy alt hesabı" = yatırımcının ayırdığı sermaye.
- Kopyalama sunucuda, oransal (equity-to-equity).
- Ücretler cTrader modelinde; tavanlar yerel regülasyona göre ayarlanır.
- Equity stop ve "takibi durdur" tek tık. Ücret şeffaflığı ilk ekranda.

---

## 15. Sunucu ve back-office

### 15.1 MT5 sunucu mimarisi [3P][R]
- Bileşenler:
  - **Main** (Trade) Server,
  - **History** Server,
  - **Access** Server'lar (istemci giriş noktası, ölçek ve DDoS dağıtımı),
  - **Backup / Failover**.
- Kurulum sırası (B2Broker kurulum hizmeti tanımı): Main ve History dağıtılır, Access'ler bağlanır, sonra semboller, routing, gateway'ler ve yedekleme yapılandırılır [3P].
- **Gateway**'ler:
  - Platform ile sağlayıcı yazılımı arasında yerel bağlantı; piyasa verisi alır ve işlem yürütür.
  - Broker gateway üzerinde marj kuralları, işlem talebi routing'i, enstrüman yeniden adlandırma ve fiyat dönüştürme ayarlayabilir [R].
  - Yerel gateway listesinde **LMAX Global**, Integral, Cboe FX, Euronext FX, Currenex, FXCM Pro, Swissquote, Alpari var [R].
- **MT5 Administrator**: LP'ler, gateway'ler, köprüler ve routing yapılandırması (iş ilanı tanımından) [3P]. Semboller, gruplar, yöneticiler ve izinler, seanslar [?].
- **MT5 Manager**: hesap açma ve kapama, bakiye ve kredi, işlem gözetimi, dealing, raporlar, risk/exposure [?].
- **Manager API**: yönetici ve admin araçlarının fonksiyon ve veri yapıları; C++ ve .NET. Topluluk paketleri: PyPI `MT5Manager`, PHP SDK, JSON sarmalayıcılar [3P].
- **Web API**: REST komut ve veri + WebSocket akış (B2Broker açıklaması) [3P].
- Lisans: White Label / tam lisans. Fiyatlar kamuya açık değil **[?]**.

### 15.2 MT5 grup ve sembol yapılandırması [3P/?]
- **Group**: kullanıcıların varsayılan ayarları ve izinleri. Margin Call ve Stop Out seviyeleri API modelinde alan olarak var [3P: dev4traders].
- Diğer grup alanları (genel bilgi **[?]**): mevduat para birimi, kaldıraç, marj modu, hedged margin, komisyon tanımları (deal / volume / turnover, kademeli), sembol bazında override (spread farkı, hacim limitleri, izinler).
- **Symbol** alanları **[?]**:
  - kontrat büyüklüğü, tick size ve tick value, digits,
  - marj hesap tipi (Forex, CFD, CFD Leverage, Futures, Exchange),
  - quote ve trade seansları,
  - swap tipi (puan / % / para) ve 3 günlük swap günü,
  - yürütme modu, izinli doldurma politikaları, expiration türleri,
  - min, max ve adım hacim, stop level, freeze level.

### 15.3 cBroker (cTrader back office) [R]
- Hesap yönetimi, emir/deal/pozisyon raporları, cTrader UI/UX yapılandırması. Hesap yönetimi, enstrüman oluşturma, risk ve raporlama **eklentisiz** gelir.
- Ayarların çoğu **grup, enstrüman ve hesap** düzeyinde uygulanabilir (üç katmanlı override).
- Sürüm notları:
  - **9.4**: feed symbol settings için toplu silme (likidite feed yönetimi); **symbol split** (hisse bölünmesi) desteği.
  - **9.5**: çok yüksek kaldıraç, **toplu zorla kapatma**, **marj bypass**.
  - **9.6**: **Sessions** uygulaması (oturum ID, hesap, IP vb. izleme); symbol split'te grafik yeniden hesaplama; hesap uygulamasında performans iyileştirmesi.
- FIFO netting hesap türü back-office'te yapılandırılır [R].
- IB/partner modülü, KYC ve CRM entegrasyonu, LP köprüsü ayrıntıları bu turda doğrulanamadı **[?]**.

### 15.4 Back-office ekranları: fxvps.ai için gerekli liste
Rakiplerin ekranlarından çıkardığımız, bizim için önerilen ekranlar:

| Ekran | İçerik | Rakip karşılığı | Sürüm |
|---|---|---|---|
| Hesaplar | Ana hesap → alt hesaplar ağacı; durum, grup/plan, kaldıraç, bakiye, equity, marj seviyesi; arama ve filtre | MT5 Manager Accounts, cBroker Accounts | v1 |
| Hesap ayrıntısı | Pozisyonlar, emirler, deals, bakiye hareketleri, oturumlar (IP), notlar, KYC durumu | Manager + cBroker Sessions | v1 |
| Bakiye işlemleri | Yatırma, çekme, kredi, düzeltme (çift onay, gerekçe zorunlu) | Manager balance ops | v1 |
| Planlar/Gruplar | Kaldıraç, stop-out/margin call, komisyon, markup, swap çarpanı, izinli semboller | MT5 Groups, cBroker groups | v1 |
| Semboller | LP'den otomatik içe aktarma; override (digits, min/max/step lot, seans, swap, marj %) | MT5 Symbols, cBroker symbols | v1 |
| Seanslar/Tatiller | Quote ve trade saatleri, tatil takvimi | MT5 sessions, cBroker 9.6 Sessions (oturum farklı anlamda) | v1 |
| Canlı exposure | Sembol bazında net alt hesap pozisyonu ve LP pozisyonu, uyumsuzluk alarmı | MT5 Exposure | v1 |
| Risk monitörü | Marj seviyesi en düşük hesaplar, stop-out kuyruğu, büyük emirler | Manager risk | v1 |
| LP bağlantıları | FIX oturum durumu (MD/Trading), gecikme, reddedilen emirler, sequence numaraları | Administrator gateways | v1 |
| Emir denetimi | Order → LP ExecutionReport eşlemesi, slipaj dağılımı | — | v1 |
| Raporlar | Günlük ekstre, P/L, komisyon, swap, hacim, CSV/PDF | Report Server, cBroker reports | v1 |
| Audit log | Tüm yönetici eylemleri, değiştirilemez | [?] | v1 |
| Toplu işlemler | Toplu zorla kapatma, toplu grup değiştirme | cBroker 9.5 | v2 |
| Routing kuralları | A/B/hibrit kural motoru | MT5 Administrator routing | Sonra |
| IB/Affiliate | Hiyerarşi, rebate kuralları, ödemeler | cBroker IB [?], MT5 için 3. parti CRM | v2 |
| Copy yönetimi | Stratejiler, ücretler, HWM hesapları | cTrader Copy broker ayarları | v2 |

### 15.5 Risk modeli kavramları

| Kavram | Açıklama | fxvps.ai |
|---|---|---|
| A-book | İşlem LP'ye (LMAX) aktarılır | v1 **yalnız A-book** |
| B-book | Broker karşı taraf olur | Lisans ve regülasyon gerektirir. v1 dışında, hukuki karar |
| Hibrit | Kurala göre bölme (hesap skoru, sembol, hacim) | Sonra |
| Ön-işlem kontrolleri | Marj yeterliliği, max lot, max açık pozisyon, fat-finger fiyat bandı, sembol seansı | v1 zorunlu |
| Stop-out | Sunucu tarafında, tik bazlı | v1 zorunlu |
| Negatif bakiye koruması | Sıfırlama politikası | v1 |
| Toxic flow | Latency arbitrajı, haber scalping'i | v2 analiz |

---

## 16. Likidite tarafı: FIX ve LMAX

- MT5 yerel gateway listesinde **LMAX Global** var [R]. Bu, LMAX'in FX perakende platformlarına bağlanan yerleşik bir LP olduğunu doğruluyor.
- LMAX FIX ayrıntıları (bu turda resmi LMAX dokümanı **açılamadı**; aşağıdakiler arama özetinden, kaynak belirsiz **[?]**):
  - FIX 4.4.
  - Market data abonelikleri: `PriceDepth` ve `TradeTicker` MDBookTypes.
  - Emir tipleri: Market, Limit, Iceberg (MaxShow), Dark Limit.
  - TimeInForce: IOC, FOK, DAY (spot).
  - Stop emirlerin LP'de desteklenip desteklenmediği belirsiz.
- **Tasarım çıkarımları (önerimiz):**
  1. SL, TP, stop, stop-limit ve trailing emirleri **bizim OMS'te** tutulur. Tetiklenince LP'ye **IOC limit** (max slipaj ile) ya da market olarak gönderilir.
  2. LP'de tek veya birkaç ana hesap vardır, alt hesaplar bizim defterimizdedir. Her fill'in hangi alt hesaplara dağıtılacağı (allocation) ve kısmi doldurma kuralları kritik bir tasarım konusu.
  3. FIX oturumları ayrılır: piyasa verisi ve işlem (cTrader QUOTE/TRADE ayrımının LP tarafındaki eşi).
  4. Sequence ve resend yönetimi, drop-copy ile mutabakat, gün sonu pozisyon eşleştirme.

---

## 17. UX dersleri

### 17.1 MT5'ten alınacaklar
1. **Order / deal / position** ayrımı: denetlenebilir muhasebe ve geçmiş.
2. Hesap düzeyinde hedging/netting bayrağı.
3. Doldurma politikası matrisi (yürütme modu → izinli politikalar). FIX TimeInForce ile birebir eşleşir.
4. Uyarılarda tekrar aralığı, tekrar sayısı ve bitiş zamanı parametreleri.
5. Takvim verisinin otomasyona API ile açılması.
6. Kullanıcının herhangi bir öğeye kısayol atayabilmesi.

### 17.2 MT5'ten kaçınılacaklar
1. İstemci tarafı trailing stop. Kullanıcıların çoğu terminal kapanınca çalışmadığını bilmiyor (forum şikayetleri) [R].
2. 10'dan fazla sekmeli Toolbox: işlem ve içerik sekmeleri karışık.
3. Web sürümün masaüstünden açıkça zayıf olması (31'e 38 gösterge, 23'e 44 nesne) [R].
4. Yerel profiller: cihazlar arası tutarlılık yok.
5. Copy trading için abonenin terminalinin ya da VPS'in açık kalma zorunluluğu.
6. 31 karakterlik yorum sınırı gibi eski kısıtlar.

### 17.3 cTrader'dan alınacaklar
1. Bulut Workspaces; web = masaüstü.
2. Sürekli açık sembol paneli (ASP): sentiment, pazar saatleri ve geri sayım, kaldıraç, DoM tek yerde.
3. QuickTrade'in üç modu ve görünür mod düğmesi.
4. Sunucu tarafı trailing, break-even ve SL tetikleme yöntemleri.
5. Grafik modları (Multi/Single/Free) ve Detach/Reattach.
6. Copy'de ücret şeffaflığı ("Start copying" düğmesinde).
7. Son kullanıcıya açık Open API ve FIX.
8. Back-office'te grup, enstrüman ve hesap üç katmanlı override.

### 17.4 cTrader'dan kaçınılacaklar
1. Mobilde QuickTrade'in yanlışlıkla tetiklenmesi (topluluk şikayeti) [R].
2. Özel zaman dilimi eksikliği (topluluk istekleri, Store eklentileri) [R].
3. .NET Framework / .NET 6 ikiliği gibi geçiş yükleri [R].

### 17.5 fxvps.ai'ye özgü öneriler
- **Risk çubuğu her zaman görünür**: marj seviyesi, serbest marj, "stop-out'a kalan" (para ve % olarak).
- **Emir önizlemesi**: VWAP doldurma tahmini + toplam maliyet (spread + komisyon + gecelik swap) + işlem sonrası marj seviyesi.
- **Gecikme göstergesi**: istemci ↔ sunucu ↔ LP RTT ve son tik yaşı.
- **AI asistan** (v2): doğal dilden emir bileti taslağı çıkarır. Asla otomatik göndermez; kullanıcı onayı zorunlu.
- **Alt hesap yöneticisi**: bakiye dağıtımı, alt hesap başına risk limiti, salt okuma paylaşım bağlantısı.
- **Komut paleti**: sembol, eylem ve ayar araması tek yerde.
- **Erişilebilirlik**: yüksek kontrast teması, renk körlüğüne uygun yukarı/aşağı renkleri (yeşil/kırmızı yerine seçilebilir palet).

---

## 18. Özellik matrisi: v1 / v2 / sonra

Öncelik ölçütleri: (a) para işleminin güvenliği için zorunlu mu, (b) rakiplerle asgari eşitlik, (c) farklılaştırıcı mı.

### 18.1 Terminal

| Özellik | v1 | v2 | Sonra | Gerekçe / not |
|---|:-:|:-:|:-:|---|
| Web terminal (responsive, PWA) | ✓ | | | Ana yüzey |
| Mobil | PWA | Yerel iOS/Android | | |
| Masaüstü (Tauri sarmalayıcı, çoklu pencere) | | ✓ | | |
| Watchlist (çoklu, arama, maliyet sütunu) | ✓ | | | (b)(c) |
| Sembol paneli (ASP tarzı: ayrıntı, seans, geri sayım, swap, komisyon) | ✓ | | | (b)(c) |
| Sentiment (alt hesap verisinden) | | ✓ | | Yeterli kullanıcı gerekir |
| Grafik (Lightweight Charts + kendi katmanımız) | ✓ | | | Lisans güvenli |
| Zaman dilimleri M1–MN + özel dakika | ✓ | | | (b) |
| Heikin-Ashi | ✓ | | | |
| Tick / Renko / Range | | ✓ | | cTrader eşitliği |
| ~30 gösterge, ~20 çizim aracı | ✓ | | | MT5 web düzeyi |
| 60+ gösterge, 40+ çizim | | ✓ | | MT5 masaüstü düzeyi |
| Çoklu grafik ızgarası | ✓ | | | |
| Detach / pop-out | | ✓ | | |
| Bulut workspace | ✓ | | | (c) |
| Market / Limit / Stop / Stop-Limit | ✓ | | | (a)(b) |
| Max slipaj (market range) | ✓ | | | (a) |
| SL/TP (4 giriş modu), grafikte sürükle | ✓ | | | (a) |
| Sunucu tarafı trailing + break-even | ✓ | | | (a)(c) |
| SL tetikleme yöntemi (tek / çift tik) | | ✓ | | |
| Bracket (giriş + SL + TP) | ✓ | | | |
| OCO grupları | | ✓ | | |
| Kısmi kapatma, toplu kapatma (tümü / kârlı / zararda / yön) | ✓ | | | (b) |
| Reverse, Double Up | ✓ | | | Ucuz, cTrader eşitliği |
| Close By | | ✓ | | |
| Netting / FIFO hesap | | ✓ | | Regülasyon |
| QuickTrade (tek / çift / kapalı) | ✓ | | | (b) |
| DoM Standard + Price | ✓ | | | (b) |
| VWAP önizleme / VWAP DoM | ✓ | | | (c) |
| Uyarılar (fiyat, marj, emir olayları), sunucu tarafı | ✓ | | | (a) |
| Kanallar: Web Push, e-posta, Telegram | ✓ | | | |
| Ekonomik takvim + grafik pinleri | | ✓ | | Veri lisansı |
| Haber akışı (AI özetli) | | | ✓ | |
| Kısayollar (yeniden atanabilir), komut paleti | ✓ | | | |
| İşlem kısayolları (silahlı mod) | | ✓ | | |
| Açık / koyu / yüksek kontrast tema; 15 dil | ✓ | | | |
| Risk çubuğu, emir önizlemesi, gecikme göstergesi | ✓ | | | (c) |
| AI asistan (onaylı ticket taslağı) | | ✓ | | |
| Copy trading (sunucu tarafı, ücretli) | | ✓ | | |
| Kullanıcı REST/WS API | | ✓ | | |
| Kullanıcı FIX API (QUOTE/TRADE) | | | ✓ | |
| Otomasyon (TS/Python sandbox, bulutta çalıştır) | | | ✓ | |
| Backtest / optimizasyon | | | ✓ | |
| Replay modu | | | ✓ | |

### 18.2 Sunucu ve back-office

| Özellik | v1 | v2 | Sonra |
|---|:-:|:-:|:-:|
| FIX gateway: LMAX MD + Trading, sequence/resend, mutabakat | ✓ | | |
| Çoklu LP + agregasyon / akıllı yönlendirme | | ✓ | |
| OMS: order/deal/position, ön-işlem risk kontrolleri | ✓ | | |
| Sunucu tarafı koşullu emirler (SL/TP/stop/trailing) | ✓ | | |
| Fill allocation (LP fill'i → alt hesaplar) | ✓ | | |
| Alt hesap defteri (çift kayıt) | ✓ | | |
| Marj motoru + tik bazlı stop-out | ✓ | | |
| Negatif bakiye koruması | ✓ | | |
| Sembol içe aktarma + override (plan/sembol/hesap 3 katman) | ✓ | | |
| Seans ve tatil takvimi | ✓ | | |
| Planlar/gruplar: kaldıraç, MC/SO, komisyon, markup, swap | ✓ | | |
| Dinamik kaldıraç (hacme göre) | | ✓ | |
| Swap motoru (gecelik, 3'lü gün) | ✓ | | |
| Swap-free hesaplar | | ✓ | |
| Admin: hesaplar, bakiye işlemleri (çift onay), gözetim | ✓ | | |
| Canlı exposure + LP mutabakat alarmı | ✓ | | |
| Raporlar (CSV/PDF), audit log | ✓ | | |
| Oturum ve IP izleme | ✓ | | |
| Toplu zorla kapatma, toplu grup değişimi | | ✓ | |
| KYC/CRM (mevcut mt5forexvps altyapısının genişletilmesi) | ✓ | | |
| Ödemeler (kurucu onaylı akış) | ✓ | | |
| IB / affiliate | | ✓ | |
| Copy trading motoru (HWM ücret) | | ✓ | |
| Toxic flow analizi | | ✓ | |
| Hibrit A/B routing kural motoru | | | ✓ (hukuki onayla) |
| White-label | | | ✓ |
| Eklenti / kural motoru (Server API eşdeğeri) | | | ✓ |

---

## 19. Açık sorular ve doğrulama listesi

| # | Soru | Durum | Nasıl doğrulanır |
|---|---|---|---|
| 1 | MT5 DoM kısayolu Alt+B resmi listede mi? | [?] Resmi özette Ctrl+B = Objects List | metatrader5.com hotkeys sayfasını engelsiz ağdan oku |
| 2 | cTrader TradeWatch sekme adları | [?] | help.ctrader.com/ctrader/ arayüz sayfası |
| 3 | cTrader zaman dilimi ve gösterge sayısı | [?] | help.ctrader.com glossary/charting |
| 4 | Market Range'in tam semantiği | [?] | help.ctrader.com/ctrader/trading/orders |
| 5 | cTrader Reverse / Double Up / toplu kapatma seçenekleri | [?] | Positions yardım sayfası |
| 6 | MT5 web terminalinde EA/özel gösterge yokluğu | [?] | metatrader5.com web-trading/features |
| 7 | LMAX FIX: stop emir desteği, GTC, derinlik seviyesi | [?] | LMAX FIX spesifikasyonu (müşteri portalı) |
| 8 | TradingView Advanced Charts'ın giriş arkası terminalde ücretsiz kullanımı | [?] | TradingView satış ekibi |
| 9 | Alt hesap modelinin hangi yetki alanında broker lisansı gerektirdiği | Hukuki | Hukuk danışmanı |
| 10 | Ekonomik takvim veri sağlayıcısı ve lisansı | Açık | Tedarikçi teklifleri |
| 11 | cBroker IB modülü ve KYC entegrasyon yüzeyi | [?] | Spotware satış / doküman |
| 12 | MT5 Python paketinin güncel durumu | [?] | mql5.com/en/docs/python_metatrader5 |

**Not:** Bu turda metatrader5.com ve help.ctrader.com sayfaları ağ politikası nedeniyle doğrudan açılamadı. [R] bilgiler resmi sayfaların arama özetlerinden geliyor. Engelsiz bir ortamda (ör. kurucu PC'si) bu sayfalar tek tek okunup işaretler güncellenmeli.

---

## 20. Kaynaklar

Hepsi bu turdaki aramalarda gerçekten döndü. Gövdesi okunamayanlar [R] ya da [3P] olarak işaretlendi.

**MetaTrader 5 (resmi alan adları)**
- Trading Basic Principles (hedging/netting, order/deal/position): https://www.metatrader5.com/en/terminal/help/trading/general_concept
- Executing Trades (New Order alanları): https://www.metatrader5.com/en/terminal/help/trading/performing_deals
- Depth of Market (terminal yardımı): https://www.metatrader5.com/en/terminal/help/trading/depth_of_market.md
- Hot Keys: https://metatrader5.com/en/terminal/help/start_advanced/hotkeys
- MetaEditor hot keys: https://www.metatrader5.com/en/metaeditor/help/workspace/hotkeys
- Journal: https://www.metatrader5.com/en/terminal/help/start_advanced/journal.md
- Charts analysis: https://www.metatrader5.com/en/terminal/help/charts_analysis.md
- Technical analysis (38 / 44 / 21): https://www.metatrader5.com/en/trading-platform/technical-analysis
- Trade alerts: https://www.metatrader5.com/en/trading-platform/alerts
- Android — Types of Orders: https://www.metatrader5.com/en/mobile-trading/android/help/trade/general_concept/order_types
- iPhone — Types of Orders: https://www.metatrader5.com/en/mobile-trading/iphone/help/trade/general_concept/order_types
- Android — Fill Policy: https://www.metatrader5.com/en/mobile-trading/android/help/trade/general_concept/fill_policy
- iPhone — Fill Policy: https://metatrader5.com/en/mobile-trading/iphone/help/trade/general_concept/fill_policy
- iPhone — Placing Pending Orders: https://www.metatrader5.com/en/mobile-trading/iphone/help/trade/pending_place
- Android — Instant Execution: https://www.metatrader5.com/en/mobile-trading/android/help/trade/positions_manage/open_positions/position_instant
- Web platform resmi sürüm + DoM: https://metatrader5.com/en/news/1355
- Web platform release notes 1359: https://www.metatrader5.com/en/releasenotes/terminal/1359
- Web trading features: https://www.metatrader5.com/en/trading-platform/web-trading/features.md
- Release notes build 261 (trailing stop): https://www.metatrader5.com/en/releasenotes/terminal/261
- Build 2005 (Economic Calendar, services): https://www.metatrader5.com/en/releasenotes/terminal/1920
- Signals — How to Subscribe: https://metatrader5.com/en/terminal/help/signals/signal_subscriber
- MetaTrader 5 for brokers: https://www.metatrader5.com/en/brokers.md
- ECN and liquidity providers (gateway listesi, LMAX Global): https://www.metatrader5.com/en/stocks-ecns/liquidity_providers_ecns
- MQL5 forum — trailing stop sorunu: https://www.mql5.com/en/forum/217737
- MQL5 forum — Gateways & Connectivity: https://www.mql5.com/en/forum/13560/641935
- MQL5 book — calendar: https://www.mql5.com/tr/book/advanced/calendar
- MetaQuotes — mobil sinyal aboneliği: https://metaquotes.net/en/company/news/5320

**cTrader / Spotware (resmi alan adları)**
- Depth of Market: https://help.ctrader.com/trading-with-ctrader/depth-of-market/
- Hotkeys (Windows): https://help.ctrader.com/ctrader/miscellaneous/hotkeys
- Hotkeys (Web): https://help.ctrader.com/ctrader-web/miscellaneous/hotkeys
- Hotkeys (Mac): https://help.ctrader.com/ctrader-mac/miscellaneous/hotkeys/
- Orders: https://help.ctrader.com/ctrader/trading/orders
- Orders (Web): https://help.ctrader.com/ctrader-web/trading/orders
- Protections: https://help.ctrader.com/ctrader/trading/protections/
- StopTriggerMethod (Automate API): https://help.ctrader.com/ctrader-automate/references/Trading/StopTriggerMethod/
- Settings: https://help.ctrader.com/ctrader/interface/settings
- Settings (Web): https://help.ctrader.com/ctrader-web/interface/settings
- Market Watch: https://help.ctrader.com/ctrader/interface/market-watch/
- Active Symbol Panel: https://help.ctrader.com/ctrader/interface/active-symbol-panel
- Active Symbol Panel (Web): https://help.ctrader.com/ctrader-web/interface/active-symbol-panel
- Chart Modes: https://help.ctrader.com/ctrader/charts/chart-modes/
- Chart toolbar: https://help.ctrader.com/ctrader/charts/chart-toolbar
- Chart trading (Mac): https://help.ctrader.com/ctrader-mac/trade/chart-trading/
- Charting glossary: https://help.ctrader.com/knowledge-base/glossary/charting/
- Mobile iOS charts: https://help.ctrader.com/ctrader-mobile-ios/trade/charts/
- Mobile iOS symbol overview: https://help.ctrader.com/ctrader-mobile-ios/trade/symbol-overview/
- cBots: https://help.ctrader.com/ctrader-algo/cbots/
- cBots introduction: https://help.ctrader.com/ctrader-algo/documentation/cbots/
- Algo glossary: https://help.ctrader.com/knowledge-base/glossary/algo/
- Open API proxies/endpoints: https://help.ctrader.com/open-api/proxies-endpoints/
- FIX API: https://help.ctrader.com/fix/
- FIX spec (Rules of Engagement): https://help.ctrader.com/fix/specification/
- cTrader Copy fees: https://help.ctrader.com/ctrader-copy/fees-calculation
- cTrader FAQ — copy trading: https://ctrader.com/faq/copy-trading
- cTrader FAQ — cBots: https://ctrader.com/faq/cbots
- Spotware — cTrader FIX API: https://www.spotware.com/ctrader-fix-api
- Spotware — hedging & netting: https://www.spotware.com/news/ctrader-supports-hedging-and-netting-accounts/
- Spotware — FIFO netting: https://spotware.com/ctrader/back-office/fifo-netting-trading-account/
- Spotware — cTrader Copy live: https://www.spotware.com/news/ctrader-copy-a-new-copy-trading-service-is-live/
- Spotware — cTrader Desktop 4.2 (.NET 6): https://spotware.com/news/ctrader-desktop-4-2
- Spotware — Desktop 3.5: https://spotware.com/news/ctrader-desktop-3-5-provides-more-usability-with-a-new-look
- Spotware — Web 3.0: https://spotware.com/news/ctrader-web-3-0-introduces-all-in-one-experience
- Spotware — trailing (Nisan 2016 güncellemesi): https://www.spotware.com/news/ctrader-for-windows-updates-april-2016/
- Spotware — cBroker: https://spotware.com/products/brokers/cbroker
- cBroker 9.4: https://www.spotware.com/news/spotware-unveils-cbroker-94/
- cBroker 9.5: https://www.spotware.com/news/cbroker-9-5-release/
- cBroker 9.6: https://www.spotware.com/news/cbroker-96-release/
- Topluluk — Advanced Stop Loss: https://community.ctrader.com/forum/announcements/1317/
- Topluluk — trailing: https://community.ctrader.com/forum/ctrader-support/9063
- Topluluk — QuickTrade yanlış tetikleme: https://community.ctrader.com/forum/ctrader-mobile/23081
- Topluluk — özel zaman dilimi isteği: https://community.ctrader.com/forum/suggestions/39676
- Open API referansı (ReadTheDocs): https://spotware-open-api.readthedocs.io/en/latest/references/
- OpenApiPy: https://github.com/spotware/OpenApiPy

**Üçüncü parti**
- Finance Magnates — cTrader 3 DoM tipi: https://www.financemagnates.com/forex/technology/ctrader-now-offers-3-types-of-depth-of-market/
- FxPro — single/double click: https://FXPro.com/help-section/faq/ctrader-platform/how-can-i-set-up-single-click-or-double-click-trading
- FxPro — MT4/MT5 hotkeys: https://fxpro.com/help-section/education/beginners/articles/list-of-hotkeys-for-metatrader-4-and-metatrader-5
- BlackBull — MT5 DoM kullanımı: https://blackbull.com/en/support/how-to-use-depth-of-market-on-metatrader-5-mt5/
- Deriv — MT5 toolbox: https://deriv.com/ae/academy/lessons/discovering-the-deriv-mt5-toolbox
- Pepperstone — margin call / stop out: https://pepperstone.com/en-eu/help-and-support/new-to-trading/when-does-a-margin-call-and-stop-out-occur/
- B2Broker — MT5 kurulum: https://b2broker.com/products/setup-support/
- B2Broker — MT Web API: https://b2broker.com/news/web-api-for-metatrader-how-does-it-work/
- dev4traders MT5 Manager API: https://www.mintlify.com/dev4traders/mt5-manager-api
- dev4traders Group modeli: https://www.mintlify.com/dev4traders/mt5-manager-api/api/models/group
- Kenmore — MT5 Manager JSON API: https://www.kenmoredesign.com/solutions/mt5-api-json
- PyPI MT5Manager: https://pypi.org/project/MT5Manager/5.0.4656
- Deriv geliştirici — MT5: https://developers.deriv.com/docs/mt5
- Finance Magnates — VARIANSE copy kullanımı: https://www.financemagnates.com/forex/products/a-quarter-of-varianses-ctrader-clients-now-copy-other-traders-strategies/
- TradingView ücretsiz grafik kütüphaneleri: https://www.tradingview.com/free-charting-libraries/
- TradingView charting library docs: https://in.tradingview.com/charting-library-docs/latest/introduction/
- LuxAlgo karşılaştırma (Advanced Charts lisans/fiyat iddiası): https://www.luxalgo.com/vela/compare/
- MarketFactory confluence (LMAX FIX özetlerinin muhtemel kaynağı) [?]: https://confluence.marketfactory.com/x/ux5TB
- TradingView Lightweight Charts (GitHub): https://github.com/tradingview/lightweight-charts
