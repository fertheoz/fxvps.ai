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
| Drill-down raporlar: kayma, icra süresi, pozisyon, ret analizi; **anlaşma anı kitap anlık görüntüsü** | 🟡 Etap 1-2 (sent_bid/ask); tam kitap yok |
| Düzenleyici raporlama dışa aktarımı | ✅ Etap 11 |
| MT Analytics: VaR, para birimi maruziyeti, müşteri profili | 🟡 profil/toksisite var; **VaR yok** |

### FXCubic (kural motoru derinliği + beslenti kalitesi)
| Özellik | Bizde |
|---|---|
| Kural koşul değişkenleri: hesap, grup, sembol, yön/büyüklük/tip, **hesap NOP**, **zaman penceresinde toplam hacim**, **ağ adresi (IP)**, **scalper bayrağı**, **platform bayrağı (mobil/web/API/EA/sinyal)** | 🟡 hesap/grup/sembol/lot/tip/saat/toksisite var; NOP, pencere hacmi, IP, platform, scalper yok |
| **Zamanlanmış profil geçişi** (ör. 14:30'da volatilite kipi) ve **koşula bağlı dinamik geçiş** | ⬜ |
| **Gerçek zamanlı markup API'si** (dışarıdan anlık markup/spread) | ⬜ |
| Maksimum spread kontrolü, kayma kontrolü | 🟡 kayma var; **maks spread kapısı yok** |
| Toksik işlem koruması, B-book zarar koruması | ✅ Etap 7 toksisite + limitler |
| **Arızalı LP beslentisi tespiti**: gecikme ölçümü, otomatik askıya alma, akış değiştirme | 🟡 sessiz-LP + sapma koruması var; **gecikme tespiti yok** |
| LP fiyat geçmişini görsel karşılaştırma | ⬜ |
| Sağlayıcı QoS izleme, performans alarmı | 🟡 LP performans raporu var; canlı QoS alarmı yok |
| Yatay ölçek, failover tatbikatı, paralel göç | ✅ Etap 10 + mavi/yeşil |

### oneZero Hub (fiyatlama/spread kontrolleri + depo yönetimi + veri)
| Özellik | Bizde |
|---|---|
| **Hacim bantları (Volume Bands)**, **hedef spread**, **min/maks spread** kontrolleri | ⬜ |
| Auto Hedge (mikro/makro, sembol+grup+limit bazlı tetik) | ✅ Etap 7 |
| Quote Filtering (piyasa dışı kotasyon süzme) | ✅ sapma koruması |
| **Depo hacim kontrolleri** (işlem/dönem başına B-book hacim tavanı; ani toksik dalga) | 🟡 net limit var; **hacim/dönem tavanı yok** |
| **Algoritmik Fiyatlama Modülü** (kendi algoritman Hub içinde koşar) | ⬜ |
| **Data Source**: kotasyon+işlem verisi ambarı → analitik/BI | ⬜ (journal var, analitik ambar yok) |
| EcoSystem: 200+ LP/bağlayıcı, DMA, takas | ⬜ (LMAX + SIM; bağlayıcı sayısı iş geliştirme konusu) |
| Margin Engine, MetaTrader barındırma | ✅ / ✅ (10-11 MT5 köprü) |

### MahiFX / MahiMarkets (fiyat oluşturma + içselleştirme; banka seviyesi)
| Özellik | Bizde |
|---|---|
| **Fiyat oluşturma**: başkasının likiditesini toplamak değil, **kendi oranını** üretmek | ⬜ (markup = agregasyon + puan) |
| **Portföy bazlı skew**: envanteri azaltan yöne fiyatı kaydırma | ⬜ |
| **Last look** (tutma süresi + ret eşiği) — API/kurumsal müşteriye karşı | ⬜ |
| **VaR bazlı ve süre sınırlı hedge** (riski kademeli, iştah kadar çıkar) | 🟡 eşik hedge var; VaR/süreli yok |
| Öngörücü sinyal: **koordineli işlem (sürü) tespiti**, oranı savunma | ⬜ |
| MFX Echo: tik "firehose", **likiditeyi zamanda 2D/3D görselleştirme**, işlem simülasyonu (CSV yükle) | ⬜ |
| Müşteri başına oran özelleştirme | ✅ grup/kural profili |
| Gerçek zamanlı VaR/PnL/hacim panosu | 🟡 PnL/hacim var, VaR yok |

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

## Kaynaklar
PrimeXM: https://primexm.com/xcore/solutions/ · https://primexm.com/xcore-aggregation/ ·
FXCubic: https://fxcubic.com/bridging-and-aggregation-solutions-for-fx-brokers/ · https://fxcubic.com/the-fxcubic-bridge-performance-flexibility-support-for-modern-brokers/ ·
oneZero: https://www.onezero.com/retail-brokers/ · https://www.onezero.com/algorithmic-pricing-module/ · https://www.onezero.com/company/news/onezero-introduces-sophisticated-new-risk-management-tools-to-power-clients-growth/ ·
MahiMarkets: https://mahimarkets.com/ · https://blog.mahimarkets.com/blog/benefits-of-mfx-compass · https://www.financemagnates.com/forex/technology/mahifx-launches-price-explain-tool-mfx-echo/
