# Bridge özellik havuzu — MahiFX · PrimeXM · FXCubic · oneZero'dan kısayol

Kurucu kararı (9 Ekim 2026): "Bağdat'ı yeniden keşfetmek yerine kısayol":
dört köklü firmanın **en önemli ve en popüler** özellikleri ayrı ayrı
çıkarılır, bizde olanla harmanlanır, kalan boşluklar **bir PR = bir parça**
yürüyüş politikasıyla kapatılır. Teknoloji seçimi "en ileri stack" ilkesine
göre, ama kutuda çalışan ve tek kişinin işletebildiği araçlarla.

Durum işaretleri: ✅ var · 🟡 kısmen · ⬜ yok. "Var" satırları ilgili etabı
gösterir (08-bridge-yol-haritasi.md, 12-eksik-parcalar-plan.md).

## 1. Kaynak firmalar — öne çıkan özellikleri

### PrimeXM XCore (agregasyon + yönlendirme + risk; MT4/MT5 bridge standardı)
| Özellik | Bizde |
|---|---|
| Çok katmanlı en iyi alış/satış agregasyonu, sınırsız likidite havuzu, VWAP kitap süpürme | ✅ Etap 6 (Priority/BestPrice/VWAP/RR) |
| **Katmanlı markup** (band başına ayrı markup, tek birleşik kitap üstünde), sınırsız markup profili, sembol+zaman dilimi bazlı | 🟡 tek `markup_points` + kural profili; band/zaman yok |
| Akıllı yönlendirme kuralları: müşteri/sembol/emir tipi/büyüklük/hedef LP | ✅ Etap 5 kural motoru |
| Maruziyet limitleri: global / sembol / **para birimi** / zaman dilimi | 🟡 sembol/toplam/müşteri var; para birimi + zaman dilimi yok |
| Müşteri akışı netleştirme + limit üstü **otomatik LP'ye taşırma (overflow)** ve taşırma izi | ✅ Etap 7 `hedge_excess` |
| **Haber/volatil dönem** risk kısıtı | ⬜ takvim var (madde 10), kısıt bağlı değil |
| Konsoldan **elle LP'ye risk atma** (tek tık hedge) | ⬜ |
| Gerçek zamanlı panolar, LP/bağlayıcı durumu, geçmiş log erişimi | ✅ Etap 9 + nöbet |
| **Özel otomatik alarm kuralları** (e-posta) | 🟡 sabit alarm seti + Telegram; kural tanımı yok |
| **Yapılandırma değişikliği doğrulaması** | 🟡 denetim izi var; önizleme/doğrulama yok |
| Drill-down raporlar: kayma, icra süresi, pozisyon, ret analizi; **anlaşma anı kitap anlık görüntüsü** | ✅ #218 gönderim anı kitabı (ilk 5, LP başına) + #199 satır detayı |
| Düzenleyici raporlama dışa aktarımı | ✅ Etap 11 |
| MT Analytics: VaR, para birimi maruziyeti, müşteri profili | 🟡 profil/toksisite var; **VaR yok** |

### FXCubic (kural motoru derinliği + beslenti kalitesi)
| Özellik | Bizde |
|---|---|
| Kural koşul değişkenleri: hesap, grup, sembol, yön/büyüklük/tip, **hesap NOP**, **zaman penceresinde toplam hacim**, **ağ adresi (IP)**, **scalper bayrağı**, **platform bayrağı (mobil/web/API/EA/sinyal)** | 🟡 hesap/grup/sembol/lot/tip/saat/toksisite var; NOP, pencere hacmi, IP, platform, scalper yok |
| **Zamanlanmış profil geçişi** (ör. 14:30'da volatilite kipi) ve **koşula bağlı dinamik geçiş** | ⬜ |
| **Gerçek zamanlı markup API'si** (dışarıdan anlık markup/spread) | ✅ #213 `PUT /v1/pricing/markup` (TTL) |
| Maksimum spread kontrolü, kayma kontrolü | ✅ #203 maks spread kapısı + kayma |
| Toksik işlem koruması, B-book zarar koruması | ✅ Etap 7 toksisite + limitler |
| **Arızalı LP beslentisi tespiti**: gecikme ölçümü, otomatik askıya alma, akış değiştirme | ✅ #209 gecikme ölçümü + askıya alma; sessiz-LP + sapma |
| LP fiyat geçmişini görsel karşılaştırma | 🟡 #214 tik ambarı + likidite haritası (LP başına karşılaştırma grafiği yok) |
| Sağlayıcı QoS izleme, performans alarmı | ✅ #209 `lp_slow` alarmı + LP performans raporu |
| Yatay ölçek, failover tatbikatı, paralel göç | ✅ Etap 10 + mavi/yeşil |

### oneZero Hub (fiyatlama/spread kontrolleri + depo yönetimi + veri)
| Özellik | Bizde |
|---|---|
| **Hacim bantları (Volume Bands)**, **hedef spread**, **min/maks spread** kontrolleri | ✅ #203 |
| Auto Hedge (mikro/makro, sembol+grup+limit bazlı tetik) | ✅ Etap 7 |
| Quote Filtering (piyasa dışı kotasyon süzme) | ✅ sapma koruması |
| **Depo hacim kontrolleri** (işlem/dönem başına B-book hacim tavanı; ani toksik dalga) | ✅ #207 patlama tavanı (hesap/sembol, N dk) |
| **Algoritmik Fiyatlama Modülü** (kendi algoritman Hub içinde koşar) | ✅ #215 formül dili + sandbox + hot swap |
| **Data Source**: kotasyon+işlem verisi ambarı → analitik/BI | ⬜ (journal var, analitik ambar yok) |
| EcoSystem: 200+ LP/bağlayıcı, DMA, takas | ⬜ (LMAX + SIM; bağlayıcı sayısı iş geliştirme konusu) |
| Margin Engine, MetaTrader barındırma | ✅ / ✅ (10-11 MT5 köprü) |

### MahiFX / MahiMarkets (fiyat oluşturma + içselleştirme; banka seviyesi)
| Özellik | Bizde |
|---|---|
| **Fiyat oluşturma**: başkasının likiditesini toplamak değil, **kendi oranını** üretmek | ⬜ (markup = agregasyon + puan) |
| **Portföy bazlı skew**: envanteri azaltan yöne fiyatı kaydırma | ✅ #204 |
| **Last look** (tutma süresi + ret eşiği) — API/kurumsal müşteriye karşı | ✅ #208 |
| **VaR bazlı ve süre sınırlı hedge** (riski kademeli, iştah kadar çıkar) | 🟡 eşik hedge var; VaR/süreli yok |
| Öngörücü sinyal: **koordineli işlem (sürü) tespiti**, oranı savunma | ⬜ |
| MFX Echo: tik "firehose", **likiditeyi zamanda 2D/3D görselleştirme**, işlem simülasyonu (CSV yükle) | ⬜ |
| Müşteri başına oran özelleştirme | ✅ grup/kural profili |
| Gerçek zamanlı VaR/PnL/hacim panosu | 🟡 PnL/hacim var, VaR yok |

## 0. parça — LP'de yatan emirler (kurucu kararı, 9 Ekim)

**Sorun.** TP/limit bizim fiyat çizgimizde tetikleniyor, LP'ye o anda piyasa
emri gidiyor. İğne ucunda LP doldurmazsa (8 Ekim XAU/USD: devir anında LMAX
kitabı yoktu, emir SIM'e düştü) ya müşteri haksız kârla kapanır ya da
"fiyat oraya gitti, neden kapanmadı/açılmadı, spread'i siz açıyorsunuz" olur.

**Kural.** A-book'ta müşterinin TP'si ve bekleyen limit emri, **LP'de gerçek
GTC limit emri** olarak durur. Dolum LP'den gelir; müşteri fiyatı = LP dolumu
+ markup. LP işlem yapmadıysa bizde de hiçbir şey olmaz; müşteriye cevap:
"emriniz LMAX'ta duruyordu, LMAX o fiyattan işlem yapmadı".

**Kapsam:** v0 — A-book pozisyon TP'si + bekleyen **limit** girişleri (GTC
limit). v1 (#193) — SL ve **stop** girişleri GTC **stop** (tetik LMAX'ta,
dolum LMAX'ın tetik anı fiyatı). Stop-limit girişi tetik kipinde kalır (v2). B-book'a dokunulmaz (orası için derinlik-VWAP tetik,
parça 1 ile).

**Motor (oms, journal'lı, replay güvenli)**
- `GroupConfig.lp_resting` (varsayılan kapalı; grup başına açılır).
- `LpOrder.resting` + `LpOrder.position` (TP için; çocuk emir dolumda
  yaratılır) + `LpOrder.revision` (replace zinciri; ClOrdID `LP-<id>` →
  `LP-<id>-r<n>`, yeniden başlatmada deterministik).
- `Position.lp_tp`, `Order.lp_resting`: LP'deki emrin kimliği.
- Yaşam döngüsü: pozisyon açıldı/TP değişti → gönder/değiştir/iptal; elle
  kısmi kapanış → hacim değiştir; tam kapanış → iptal. Bekleyen limit: kabulde
  gönder; değiştir → replace; iptal → cancel; dolum → normal çocuk dolumu.
- LP limit fiyatı = müşteri fiyatı − markup(o yön); dolumda müşteri ≥ TP alır.
- `check_sl_tp`/`check_pending`: LP'de yatan emir için bizim tetik **çalışmaz**.
- Dolum pozisyonun serbest hacminden fazlaysa (elle kapanışla yarış): fazlalık
  broker hedge defterinde ters piyasa emriyle düzleştirilir (omnibus değişmezi).
- LP reddi/iptali → tetik kipine geri düşer (`lp_tp = None`), olay yazılır.
- `LpRouter` trait: `send` + `cancel` + `replace`; replay'de NullRouter.

**Köprü (core-engine lp_fix / lp_agg)**
- GTC Submit; Cancel/Replace `OrderCommand`'ları; icra ClOrdID'sinden kimlik
  ayrıştırma `-r<n>` ekiyle. Yatan emir yalnız **emir alan birincil LP**'ye
  gider (`orders=true`, en yüksek öncelik), o an kitap olmasa da.

**Denetçi**: GTC emri "ack geldi" sayılır (NoAnswer yalnız IOC/FOK ve ack'siz
GTC için); yatan emir uçuşta sayılmaz, uzlaştırma devam eder.

**Konsol**: Grup formu → "TP/limit LP'de yatsın" seçeneği; ekranlar
(terminalde "LP'de bekliyor" rozeti, "neden dolmadı" kaydı) v1.

**Bilinen sınırlar (v0):** yatan limit girişinde marj yalnız kabulde
denetlenir (LP dolumu bağlayıcı); aynı sembolde ters yönlü yatan emirler
LMAX'ta kendi kendine eşleşebilir (self-match; v1: netleştirilmiş tek emir);
trading süreci kapalıyken gelen icra Denetçi farkı olarak görünür.

## 2. Harmanlanmış havuz — eksik parçalar, öncelik sırasıyla

Ölçüt: broker kârına doğrudan etki × satış argümanı × bizde temel var mı.

| # | Parça | Kaynak | Ne |
|---|---|---|---|
| 1 | **Kural motoru değişkenleri** | FXCubic | NOP, pencere hacmi (N dk'da toplam lot), IP/CIDR, platform bayrağı (terminal/mobil/API/EA/copy), scalper bayrağı (FlowStats'tan), haber penceresi (takvim) |
| 2 | **Profil zamanlayıcı** | FXCubic/PrimeXM | kural seti + markup profilinin **saat/gün/haber** planı; koşullu geçiş (volatilite = spread ölçümü eşiği); kuru koşum |
| 3 | **Hacim bantları + spread kapıları** | oneZero/PrimeXM | band başına markup (0-1 / 1-5 / 5+ lot), hedef spread, min/maks spread; maks spread aşılınca emir reddi/bekletme |
| 4 | **Envanter skew'li fiyat oluşturma** | MahiFX | B-book net pozisyonuna göre müşteri fiyatını kaydır (puan/lot eğimi, tavan), grup bazında; "kendi oranımız" |
| 5 | **Süre sınırlı / VaR hedge** | MahiFX/oneZero | fazlalığı tek seferde değil TWAP dilimleriyle çıkar; sembol başına VaR (parametrik, tarihsel volatilite) ve VaR tavanı → hedge tetiği |
| 6 | **Para birimi maruziyeti + haber kısıtı** | PrimeXM | USD/EUR/… bacak bazında net; haber penceresinde limit daraltma, A-book'a zorlama |
| 7 | **Depo hacim tavanı** | oneZero | hesap/grup/sembol başına N dk'da maks B-book lot; aşınca A-book |
| 8 | **Last look (API müşterisi)** | MahiFX | API/FIX emirlerinde tutma süresi + fiyat hareket eşiği ile ret; istatistiği raporda |
| 9 | **Beslenti QoS** | FXCubic | LP gecikme ölçümü (kotasyon zaman damgası − alım), eşik üstü askıya alma, LP fiyat geçmişi karşılaştırma grafiği, QoS alarmı |
| 10 | **Elle risk atma + alarm kuralları + değişiklik doğrulama** | PrimeXM | Risk sayfasında "LP'ye at" (sembol, lot, MFA); alarm kural düzenleyici (metrik, eşik, kanal); ayar kaydetmeden önce etki önizlemesi |
| 11 | **Sürü tespiti** | MahiFX | aynı sembol/yön/saniyede çok hesap → sinyal; kural motoru girdisi + alarm |
| 12 | **Gerçek zamanlı markup API'si** | FXCubic | `/v1/pricing/markup` PUT (anahtar+MFA), TTL'li geçici markup |
| 13 | **Tik ambarı + analitik** | oneZero/MahiFX | kotasyon/derinlik/işlem → sütunlu ambar; markout, likidite-zaman haritası, VaR girdisi, "bu ayarla dün gelir ne olurdu" |
| 14 | **Algoritmik fiyatlama modülü** | oneZero/MahiFX | broker'ın kendi fiyat/skew algoritması, sandbox'ta, hot-swap |
| 14b | **Dahili eşleştirme** | Your Bourse | aynı sembolde ters yönlü müşteri akışını LP'ye gitmeden içeride eşleştir; spread kârı tamamen bizde; omnibus yalnız net fazlayı taşır; Denetçi değişmezi korunur (kurucu: "bu özellikle çok güzel") |
| 16 | **Müşteri robotları** | TradingView/MT5 | kademe 1: TradingView uyarı webhook'u → REST (`docs/15`); kademe 2: terminal içi sandbox strateji betiği + backtest sekmesi; kademe 3: sunucuda barındırma |
| 15 | **Anlaşma anı kitap anlık görüntüsü** | PrimeXM | LP emri anında birleşik 5 seviye kitap `LpOrder`'a; icra raporunda görüntüle |

## 3. Teknoloji kararları ("en ileri stack", tek kişilik işletim)

| Konu | Karar | Neden |
|---|---|---|
| Kural motoru | Mevcut sıralı kural modeli; koşullar **tipli enum** olarak büyür (DSL yok) | journal'da deterministik, replay güvenli; DSL yorumlayıcı = yeni hata yüzeyi |
| Zamanlı profiller | `Command::SetSchedule` journal'da; motor UTC saat/gün/takvim penceresiyle etkin profili seçer | swap zamanlayıcısıyla aynı kalıp (Etap 8) |
| Skew / fiyat oluşturma | Motor içinde (oms), grup başına `SkewPolicy` (eğim puan/lot, tavan) | kotasyon yolunda <1 µs; dış servis gecikme ekler |
| Algoritmik fiyatlama | **WebAssembly (wasmtime)** modülü, kotasyon başına çağrı, yakıt/süre sınırı, imzalı yükleme | sandbox + hot-swap + dil bağımsız; oneZero APM'nin karşılığı, bankaya satılabilir |
| VaR | Parametrik (EWMA volatilite) + tarihsel (tik ambarından) ikisi de; motor parametriği kullanır | parametrik ucuz ve anlık; tarihsel rapor için |
| Tik ambarı | **Parquet dosyaları + DuckDB** (kutuda, gömülü) | sıfır ek servis; sütunlu, sıkıştırmalı; ClickHouse'a geçiş yolu açık (aynı SQL) |
| Analitik görseller | Konsolda mevcut lightweight-charts; likidite-zaman haritası için canvas ısı haritası (bağımlılık yok) | |
| Last look | Motorda `Order` üzerinde `hold_until` + ret eşiği; yalnız API/FIX kaynaklı emirler | terminal müşterisine last look uygulanmaz (itibar) |
| Markup API | Mevcut admin API + kişisel anahtar (madde 2) + MFA; TTL'li geçici markup motor komutu | |
| Beslenti QoS | fix-gateway'de kotasyon `SendingTime` − alım farkı → lp_status'a `latency_ms`; aggregator `max_latency_ms` kapısı | sessiz-LP kapısıyla aynı yerde |
| Sürü tespiti | Motor dışı, `FlowStats` yanında kayan pencere sayaçları (sembol/yön/sn) | alarm + kural girdisi |

## 4. Yürüyüş

- Sıra yukarıdaki tablo; her parça: dal → kod → sınama → CI → birleştir → `dagit.sh` → canlı doğrula → bu dosyaya ✅ + günlük.
- Değişmezler: Denetçi 0 olay, SAĞLIK OK; değilse geri al. Sırlar yazdırılmaz; hiçbir şey silinmez.
- Konsolda MFA isteyen ayarlar (profil, skew, limit) kurucu kuyruğuna düşer, iş beklemez.
- Satış yüzü: her parça bittiğinde fxvps.ai özellik sayfasına bir satır (rakip adıyla değil, özellik adıyla).

## Durum günlüğü
| Tarih | Parça | Durum | PR | Not |
|---|---|---|---|---|
| 2026-10-09 | — | 📝 | — | havuz çıkarıldı; 15 parça sıralandı |
| 2026-10-09 | 0 | ✅ | #189 #186 #185 #188 #190 | LP'de yatan emirler v0: motor (TP + limit giriş, replace/cancel, fazlalık düzleştirme), köprü (GTC, revizyonlu ClOrdID), Denetçi (GTC ack), konsol grup seçeneği; canlı. Kurucu tıkı: grupta "lp" seç, LP sayfasında SIM "Emir alır" kapat. v1: Stop OrdType (SL LP'de), netleştirme, terminal rozeti + "neden dolmadı" |
| 2026-10-09 | 0 v1 | ✅ | #193 | SL ve stop girişleri LMAX'ta GTC **stop** (Stop OrdType 40=3/StopPx 99 codec→geçit→köprü→motor); trailing replace 1 sn kısıtlı; kaymaya açık kalan yalnız piyasa emirleri. Stop-limit girişi tetik kipinde (v2) |
| 2026-10-09 | 0 v1b | ✅ | #194 | Tek kalem: retry politikalı piyasa emrinde LP kısmi dolumları zincir bitene kadar tutulur, müşteriye tek dolum (VWAP+markup), tek deal/pozisyon; iç hamleler görünmez |
| 2026-10-09 | 0 v2a | ✅ | #196 | Terminalde "LP" rozeti (pozisyon SL/TP ve bekleyen emirde; proto v1.3 `lp_tp/lp_sl/lp_resting`, REST aynı); ask çizgisi varsayılan açık (eski kayıtlar bir kez açılır) |
| 2026-10-09 | 1 | ✅ | #197 | Kural değişkenleri: platform (terminal/mobile/api/bridge/copy), IP öneki/CIDR, hesap NOP, pencere hacmi (N dk'da açılan lot), scalper profili, haber penceresi (yüksek etkili olaylar motora `SetNewsTimes`); emirde platform+IP journal'da |
| 2026-10-09 | 2 | ✅ | #198 | Profil zamanlayıcı: kurallarda dakika penceresi/haftanın günleri/min spread; gruplarda zamanlı markup pencereleri + haber markup'ı |
| 2026-10-09 | 3 | ✅ | #203 | Hacim bantları (emir büyüklüğüne göre ek markup), spread tabanı/hedef (simetrik genişletme), spread tavanı (üstünde yeni piyasa emri yok, bekleyenler bekler, kapanış serbest) |
| 2026-10-09 | 4 | ✅ | #204 | Envanter skew: grup fiyatı B-book net pozisyonla kayar (puan/lot, tavan); bizi düzleştiren akış ödüllenir; havuza 14b dahili eşleştirme eklendi |
| 2026-10-09 | 13+ | 🔧 | #221 | Tik ambarı devamı: ilk 5 derinlik örneklemesi (`<gün>.depth`, 5 sn), likidite haritasında derinlik sütunları; ambardan gerçekleşen günlük σ (5 gün, 1 dk mid) saatte bir `SetVolatility` ile motora → VaR girdisi (2 gün tazelik, yoksa EWMA) |
| 2026-10-09 | 15 | ✅ | #218 | Anlaşma anı kitap görüntüsü: yönlendirici LP emrini gönderirken birleşik ilk 5 seviye + uygun LP'lerin ilk 5'i (yaş ms) agregatörde tutulur (son 5000 emir, bellek); LP icra detayında "Gönderim anındaki kitap" paneli (hangi seviye hangi LP'den) |
| 2026-10-09 | 14b | ✅ | #216 | Dahili eşleştirme: `HedgePolicy.net_delay_ms` (B-book dolumundan sonra netleştirme gecikmesi; ters akış içeride netleşir, LP'ye yalnız net fazlalık omnibus hedge olarak gider; sembol limiti 0 = tüm neti hedge et); Risk sayfasında ön ayar düğmesi + rozet; Raporlar → Analitik "Dahili eşleştirme" (müşteri lot vs LP lot, içeride eşleşen %, yakalanan) |
| 2026-10-09 | 14 | ✅ | #215 | Algoritmik fiyatlama: grup başına alış/satış kayma formülü (`risk::algo` mini dil: değişkenler spread/net/lots/vol/hour/news/markup, min/max/abs/clamp/if, and/or/not; toplam, yan etkisiz), her kotasyonda markup+skew sonrası; kaydet = hot swap; `POST /v1/pricing/algo/test` sandbox (canlı kitapta sonuç tablosu); Gruplar formunda alanlar |
| 2026-10-09 | 13 | ✅ | #214 | Tik ambarı + analitik: `<data>/ticks/<SYM>/<gün>.tick` (40 B/tik, sn'de 1 örnek, 30 gün; `CORE_TICK_SAMPLE_MS`/`CORE_TICK_KEEP_DAYS`), Raporlar → Analitik: saat bazında likidite haritası, dolum sonrası markout (+1/5/30 sn, ambardan), what-if markup geri oynatma (`/v1/analytics/*`) |
| 2026-10-09 | 12 | ✅ | #213 | Gerçek zamanlı markup API'si: `PUT /v1/pricing/markup` (grup/sembol, ± puan, TTL, sebep; groups.edit + MFA), `GET`, `DELETE /{id}`; motorda journallanan `SetTempMarkup`/`ClearTempMarkup`, grup markup'ı + zamanlayıcı + bantların üstüne eklenir, süresi dolunca düşer; Risk sayfasında kart |
| 2026-10-09 | 11 | ✅ | #212 | Sürü tespiti: aynı sembol+yönde N saniyede ≥K farklı hesap → kural değişkeni (`herdAccounts`/`herdWindowS`, örn. A-book'a zorla), `herd` uyarısı (Ayarlar → davranış eşikleri), Platform sayfasında sürü sinyalleri; motor `herd_signals()` |
| 2026-10-09 | 10b | ✅ | #211 | Elle risk atma: Risk → "LP'ye at" (sembol/yön/lot, MFA, iki aşamalı onay) → `Command::ManualHedge` hedge defterine; hedge politikası "etkiyi önizle" (sembol başına hedef/değişim/ilk emir, VaR ve para birimi limitleri); Ayarlar → Uyarı kuralları (11 ölçüt, hedef, eşik, önem; canlı okuma + kaydetmeden önizleme; `rule:<id>` uyarıları) |
| 2026-10-09 | 10a | ✅ | #210 | Hesap davranışı + platform kullanıcıları: geçit bağlantı/kimlik olay halkası (terminal WS + köprü), scalper/patlama/döngü/kaba kuvvet/IP bayrakları, `account_abuse`/`brute_force` uyarıları, pano kartı; `PlatformUser` sicili (köprü `{"t":"users"}` + JSON içe aktarma), yeni "Platform kullanıcıları" sayfası (kullanıcılar/hesap uyarıları/IP sekmeleri, satır detayı) |
| 2026-10-09 | 9 | ✅ | #209 | Beslenti QoS: gateway MD gecikmesi (alım − SendingTime, EWMA) → lp_status/LP sayfası "Besleme gecikmesi"; agregatör `maxLatencyMs` kapısı (yavaş LP askıda); `lp_slow` uyarısı (ayar: besleme yavaş eşiği, varsayılan 2000 ms) |
| 2026-10-09 | 8 | ✅ | #208 | Last look (yalnız API/bridge piyasa emirleri): tutma süresi + müşteri lehine kayma eşiği → ret; terminal/mobil/copy asla tutulmaz; tutulan emir iptal edilebilir |
| 2026-10-09 | 7 | ✅ | #207 | Depo hacim tavanı: N dakikada hesap/sembol başına açılan B-book lot; aşan akış A-book (`hedge:burst`) |
| 2026-10-09 | 6 | ✅ | #206 | Para birimi maruziyeti (bacak bazında A/B, USD) + para birimi tavanı (aşan akış A-book); haber penceresi (A-book'a zorla / yeni emirleri durdur, kapanış serbest) |
| 2026-10-09 | 5 | ✅ | #205 | Süre sınırlı hedge (TWAP: dilim lot + aralık) ve parametrik VaR (EWMA σ, tamsayı durum; sembol/toplam VaR Risk sayfasında) + VaR tavanı → kitabı hedge ile küçültür |
| 2026-10-09 | konsol | ✅ | #199 #201 | Başlığa tek tık: sırala+süz; satır detayı (LP zaman çizelgesi, deneme/ms, kayma, değerlendirme; Mutabakat deal'leri); isteğe bağlı sütunlar (Columns'ta açılır, hatırlanır), sunucudan tarih aralığı yükleme, sayfa boyutu hatırlanır, fiyatlar sembol basamağına yuvarlı, yoğunluk sütun sayısına göre |
| 2026-10-09 | FIX | ✅ | #200 | İşlem oturumu ham FIX çerçeveleri `fix-store/fixlog/<gün>.jsonl`; konsolda LP emri detayında "FIX mesajları" + kopyala (hazineci LP yazışması için; konsoldan hiçbir şey gönderilmez) |

## Kaynaklar
PrimeXM: https://primexm.com/xcore/solutions/ · https://primexm.com/xcore-aggregation/ ·
FXCubic: https://fxcubic.com/bridging-and-aggregation-solutions-for-fx-brokers/ · https://fxcubic.com/the-fxcubic-bridge-performance-flexibility-support-for-modern-brokers/ ·
oneZero: https://www.onezero.com/retail-brokers/ · https://www.onezero.com/algorithmic-pricing-module/ · https://www.onezero.com/company/news/onezero-introduces-sophisticated-new-risk-management-tools-to-power-clients-growth/ ·
MahiMarkets: https://mahimarkets.com/ · https://blog.mahimarkets.com/blog/benefits-of-mfx-compass · https://www.financemagnates.com/forex/technology/mahifx-launches-price-explain-tool-mfx-echo/
