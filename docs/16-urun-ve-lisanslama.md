# fxvps.ai — Ürün Ailesi, Modül Kataloğu ve Lisanslama Modeli

Sürüm: 1.0 — 12 Ekim 2026. Sahibi: kurucu. Bu belge, depodaki canlı yazılımı (main, #203–#222 dahil) **satılabilir, lisanslanabilir ürün modülleri** olarak adlandırır, her modülün teknoloji/kapasite/piyasa karşılığını verir ve aylık icra-politikası lisansından tam white-label/marka devrine kadar kademeli lisanslama yelpazesini tanımlar. Her modül depodaki çalışma alanına atıfla fiyatlanır; yeni modüller ek lisans sözleşmesiyle yelpazeye eklenir (bkz. §7).

---

## 1. Ürün ailesi ve adlandırma

Çatı marka: **FXVPS Nexus** (çalışma adı; "fxvps.ai Platform" ile değiştirilebilir). Modül adları İngilizce (satış), açıklamalar Türkçe.

| Kod | Modül | Kısa tanım | Depo alanı |
|---|---|---|---|
| **NX-CORE** | Nexus Core | Olay-günlüklü (journaled), yeniden oynatılabilir işlem motoru: OMS, pozisyon/defter, marjin, stop-out, A/B-book yönlendirme | `crates/oms` (8.5k satır), `crates/ledger`, `crates/money`, `crates/domain`, `services/core-engine` (24.7k) |
| **NX-RISK** | Nexus Risk | Kural motoru, hacim bantları, spread kapıları, envanter skew, VaR/TWAP hedge, para birimi maruziyeti, haber/patlama kısıtları, last look, algoritmik fiyatlama | `crates/risk` (2.7k), `crates/oms` hedge/kural bölümleri |
| **NX-BRIDGE** | Nexus Bridge | Çoklu-LP FIX geçidi + agregatör: FIX 4.2/4.4 codec/oturum, LP politikaları, beslenti QoS, LP'de yatan TP/SL (GTC), tek kalem dolum, Denetçi mutabakat | `crates/fix-codec`, `crates/fix-session`, `services/fix-gateway` (5k), `services/core-engine/src/lp_agg.rs`, `lp_fix.rs` |
| **NX-AUDIT** | Nexus Auditor ("Denetçi") | Omnibus = LP değişmezini 2 sn'de bir doğrulayan, sapmayı düzelten bağımsız denetçi; FIX ham mesaj kaydı | `services/fix-gateway/src/bin/denetci.rs`, `audit.rs`, `fixlog.rs` |
| **NX-GATE** | Nexus Gateway | Müşteri API'si: WebSocket terminal protokolü, REST (emir/pozisyon/mum), TradingView webhook, MT5 köprü oturumu, hız/oturum limitleri | `services/client-gateway` (8.8k), `crates/client-proto` |
| **NX-ID** | Nexus Identity | Kimlik servisi: parola, TOTP, passkey, API anahtarları, JWKS/OIDC, roller, hesap bağlama, kilit/kaba-kuvvet freni | `services/identity` (5.7k) |
| **NX-TERM** | Nexus Terminal | Web/PWA işlem terminali (grafik, derinlik, emir, LP rozeti, ask çizgisi), masaüstü (Tauri) ve mobil (Expo) kabukları | `apps/terminal` (15.9k), `apps/desktop`, `apps/mobile` |
| **NX-CONSOLE** | Nexus Console | Arka ofis: müşteriler, gruplar, kurallar, semboller, pozisyonlar, risk, LP, köprü, Denetçi, copy, takvim, partner, raporlar, onaylar, denetim izi, kullanıcılar/roller, ayarlar, platform kullanıcıları | `apps/backoffice` (13k), `services/core-engine/src/admin/*` |
| **NX-MT5** | Nexus MT5 Bridge | MT5 sunucu eklentisi oturumu (kurum anahtarı, hız/limit, mutabakat), platform kullanıcı sicili, kurum paneli | `services/client-gateway/src/bridge.rs`, `services/core-engine/src/admin/bridge_admin.rs`, `docs/10`, `docs/11` |
| **NX-COPY** | Nexus Copy | Copy trading: strateji kataloğu, takipçi oranları, IB/partner payları | `services/core-engine/src/admin/copy_admin.rs`, console `copy/`, `partner/` |
| **NX-DATA** | Nexus Warehouse & Analytics | Tik/derinlik ambarı, likidite haritası, markout, what-if markup, dahili eşleştirme raporu, gerçekleşen volatilite → VaR | `services/core-engine/src/ticks.rs`, `admin/volcalc.rs`, console Raporlar → Analitik |
| **NX-WATCH** | Nexus Watch | Uyarı motoru: yerleşik koşullar + kural editörü, hesap davranışı (scalper/patlama/döngü/kaba kuvvet/IP/sürü), LP QoS, webhook/Telegram, günlük rapor | `services/core-engine/src/admin/alerts.rs`, `activity.rs` |
| **NX-OPS** | Nexus Ops | Mavi/yeşil yayın, felaket kurtarma, yedek/geri yükleme tatbikatı, yük sınavı, LP simülatörü, yük üreteci | `deploy/lmax-demo/*.sh`, `services/lp-simulator`, `tools/loadgen`, `docs/04`, `docs/05`, `docs/09` |

Tamamlayıcı belgeler: mimari `docs/03`, FIX entegrasyonu `docs/02`, güvenlik `docs/06–07`, bridge özellik havuzu ve rakip karşılaştırması `docs/13`, MAS/Solid FX `docs/14`, TradingView `docs/15`, API `docs/api.md` + `docs/openapi.yaml`.

---

## 2. Modül kataloğu — teknoloji, kapasite, piyasa karşılığı

Her modül için: **ne yapar / teknoloji / kanıtlanmış kapasite / rakip karşılığı / değer bandı**. Değer bantları, kamuya açık fiyat listeleri ve 2025-26 satış anekdotlarına dayanan **sektör referans bantlarıdır**; kesin rakip fiyatı iddiası değildir.

### 2.1 NX-CORE — Nexus Core
- **Ne yapar:** Tüm komutlar journallanır (`Envelope{seq, ts, cmd}`), snapshot + yeniden oynatma ile deterministik durum; tam sayı aritmetiği (1e8 sabit nokta, `crates/money`), `State: Eq` → replay sonrası bit-bit eşitlik. Emir tipleri (market/limit/stop, IOC/GTC/GFM), SL/TP/trailing, OCO, kısmi dolum, tek kalem (zincirli kısmi dolumların VWAP'ı), netting/hedging marjin kipleri, ESMA kaldıraç presetleri, hafta sonu/haber kaldıraç pencereleri, stop-out, negatif bakiye koruması, swap/rollover, komisyon, işlem/deal bandı, istatistik (`FlowStats`, toksisite, markout).
- **Teknoloji:** Rust, tek yazar iplik + okuma replikası (raporlar), NATS ile süreç ayrımı, journal p99 bütçesi (`/v1/perf`), 1.000 müşteri / 10k emir/dk yük sınavı (`docs/05`).
- **Rakip karşılığı:** MT5 sunucu çekirdeği, cTrader, Match-Trader çekirdeği. Fark: açık kaynak-benzeri denetlenebilirlik, replay ile mutabakat, A/B-book kararının emir bazında kurala bağlanması.
- **Değer bandı:** platform çekirdeği olarak tek başına lisanslanmaz; NX-RISK + NX-BRIDGE ile "Execution Engine" paketi (bkz. §4).

### 2.2 NX-RISK — Nexus Risk
- **Ne yapar:** Yönlendirme kuralları (grup, hesap, sembol, lot, saat/gün, platform, IP, NOP, pencere hacmi, scalper, haber, spread, sürü), hibrit A/B yüzdesi, kural başına markup/kayma/kısmi dolum; grup başına hacim bantları, spread tabanı/tavanı, envanter skew'i, zamanlı/haber markup'ı, **algoritmik fiyatlama formülleri** (sandbox + hot swap, `risk::algo`); hedge politikası: sembol/toplam/hesap limitleri, HedgeExcess/SwitchToABook, oran/release, TWAP dilimleri, VaR tavanı, para birimi tavanları, haber penceresi, patlama tavanı, **dahili eşleştirme** (netleştirme gecikmesi), last look (API/köprü).
- **Rakip karşılığı:** oneZero Hub "Margin/Risk", PrimeXM XCore risk, MahiFX skew/algo, FXCubic (`docs/13` tabloları: 15 parçanın tamamı ✅).
- **Değer bandı:** oneZero Hub 10–25k $/ay, PrimeXM 5–15k $/ay + milyon başına, MahiFX 8–20k $/ay. NX-RISK tek başına 4–9k $/ay bandında konumlanır.

### 2.3 NX-BRIDGE — Nexus Bridge
- **Ne yapar:** FIX 4.4 (LMAX, canlı) ve FIX 4.2 (Solid FX/MAS hazır, `docs/14`) oturumları, TLS, mağaza, sıra/yeniden gönderim, oturum saatleri (NY DST), heartbeat, gecikme ölçümü (alım − SendingTime); çoklu LP agregasyonu (best-price/öncelik, LP başına lot/sembol politikası, yalnız-fiyat LP, sessiz-LP/sapma/QoS koruması), LP'de yatan TP/SL/limit/stop (GTC, ClOrdID revizyon zinciri), IOC yeniden deneme, tek kalem, anlaşma anı kitap görüntüsü, FIX ham kaydı (konsolda görüntüle/kopyala), ListStatus/PositionReport mutabakat komutları.
- **Rakip karşılığı:** oneZero/PrimeXM/FXCubic/Your Bourse köprüleri; MT5 Gateway'e göre farklar `docs/13` ve bu oturumun "MT5 Gateway ile ne eksik" notu.
- **Değer bandı:** Your Bourse 1,5–6k $/ay, FXCubic 3–8k $/ay, PrimeXM/oneZero üstü. NX-BRIDGE + NX-AUDIT 6–12k $/ay.

### 2.4 NX-AUDIT — Nexus Auditor (Denetçi)
- **Ne yapar:** Her 2 sn'de LP pozisyonu (AP) ile motor omnibus'unu karşılaştırır, sapmada düzeltme emri (ClOrdID ≤ 20), tutulan net kalıcı (`tutulan.json`), GTC yatan emir takibi, wire ExecID tekilliği, "0 olay" sağlık kapısı yayın betiğine bağlı. Mutabakat raporu (müşteri + markup = LP) konsolda.
- **Rakip karşılığı:** çoğu köprüde yok; PrimeXM/oneZero'da raporlama var, bağımsız düzeltici denetçi yok. **Öne çıkan satış argümanı.**
- **Değer bandı:** ek modül 1,5–3k $/ay; kurumsal müşteride "uyumluluk" kalemi.

### 2.5 NX-GATE — Nexus Gateway
- **Ne yapar:** WS protokolü v1.3 (kotasyon/derinlik/mum/hesap/emir/LP alanları), REST (`docs/openapi.yaml`): hesaplar, pozisyonlar, emirler, mumlar, idempotent `client_order_id`, API anahtarı kapsamları (read/trade), hız/oturum/IP limitleri, TradingView webhook (`docs/15`), MT5 köprü WS, Prometheus metrikleri.
- **Değer bandı:** platform paketine dahil; API/webhook erişimi müşteri başına ek ücret olabilir (§5).

### 2.6 NX-ID — Nexus Identity
- **Ne yapar:** Kayıt/e-posta doğrulama, parola politikası, kilit, TOTP (kurtarma kodları), passkey (WebAuthn), oturum/yenileme/iptal, API anahtarları (15 dk jeton değişimi), JWKS/OIDC keşfi, roller (admin/dealer/risk/support/readonly/partner), hesap bağlama, denetim izi, imza anahtarı rotasyonu, operatör CLI.
- **Rakip karşılığı:** Keycloak/Auth0 + özel broker rolleri. **Değer bandı:** pakete dahil; tek başına 1–2k $/ay.

### 2.7 NX-TERM — Nexus Terminal
- **Ne yapar:** Web/PWA terminal: TradingView benzeri grafik, derinlik, emir paneli, pozisyon/geçmiş, LP rozeti, ask çizgisi, kısayollar, i18n, tema; masaüstü (Tauri) ve mobil (Expo) kabukları; mock ve canlı API kipleri; e2e testler.
- **Rakip karşılığı:** MT5 web/masaüstü/mobil, cTrader, Match-Trader. **Değer bandı:** white-label terminal 2–6k $/ay (Match-Trader/B2Broker benzeri).

### 2.8 NX-CONSOLE — Nexus Console
- **Ne yapar:** 20 sayfa arka ofis (bkz. §1 tablosu), RBAC, dört-göz onayları, MFA zorunluluğu, canlı akış (SSE), tüm tablolarda sıralama/süzgeç/satır detayı/isteğe bağlı sütun/sunucudan tarih aralığı, CSV dışa aktarma, KYC/fonlama (USDT izleme), aylık ekstre e-postası, ESMA presetleri, tenant/markalama, i18n (EN/TR).
- **Değer bandı:** MT5 Manager/Admin karşılığı; pakete dahil, çok-tenant kurulumda tenant başına ücret.

### 2.9 NX-MT5 — Nexus MT5 Bridge
- **Ne yapar:** MT5 sunucu eklentisi için WS köprü protokolü (hello/anahtar/IP allowlist, emir, mutabakat, `users` kaydı), kurum (institution) yönetimi ve anahtar rotasyonu, platform kullanıcıları sicili, partner/kurum paneli; eklenti tarafı plan `docs/11`.
- **Değer bandı:** kurum başına 1–3k $/ay + hacim.

### 2.10 NX-COPY — Nexus Copy
- **Ne yapar:** Strateji kataloğu, takipçi oranı/limit, IB payları ve rebate planları, ödeme önizlemesi; copy kaynaklı emirler kural motorunda `platform: copy`.
- **Değer bandı:** 1–3k $/ay (B2Broker/Brokeree copier 1,5–4k $/ay).

### 2.11 NX-DATA — Nexus Warehouse & Analytics
- **Ne yapar:** Tik (40 B, sn'de 1) ve derinlik (ilk 5, 5 sn) ambarı, 30 gün saklama, saat bazında likidite haritası, dolum sonrası markout (+1/5/30 sn), what-if markup geri oynatma, dahili eşleştirme raporu, gerçekleşen günlük σ → VaR (saatlik). Yol haritası: Parquet/DuckDB, LP karşılaştırma grafiği.
- **Değer bandı:** 2–5k $/ay (oneZero Data Source / MahiFX analytics benzeri).

### 2.12 NX-WATCH — Nexus Watch
- **Ne yapar:** Yerleşik koşullar (LP düşük/yavaş, dolum oranı, gecikme sıçraması, maruziyet, stop-out, fonlama gecikmesi, hesap davranışı, sürü, kaba kuvvet) + kural editörü (11 ölçüt, canlı okuma, önizleme), webhook/Telegram, sessiz saatler, günlük operasyon raporu.
- **Değer bandı:** pakete dahil; kural editörü "Pro" özelliği.

### 2.13 NX-OPS — Nexus Ops
- **Ne yapar:** Tek kutuda mavi/yeşil yayın (`dagit.sh`), sağlık/Denetçi kapıları, yedek ve geri yükleme tatbikatı (`yedek.sh`, `geri-yukle.sh`), yük sınavı cron'u, LP simülatörü (FIX 4.4, stop/IOC/FOK), yük üreteci, felaket kurtarma planı (`docs/09`), güvenlik incelemesi (`docs/06–07`).
- **Değer bandı:** barındırma/işletme ücreti (§5) ve profesyonel hizmet gün ücreti.

---

## 3. Yetkinlik ve rakip konumlandırması (özet)

`docs/13`'teki üç karşılaştırma tablosu (MahiFX, PrimeXM, oneZero, FXCubic, Your Bourse) 12 Ekim itibarıyla: havuzdaki 15 parça + 14b + 16/1 ✅. Satış mesajı:

1. **Tek yazar-tek defter:** terminal, API, MT5 köprüsü ve copy aynı motorda; A/B kararı emir bazında, kural ile.
2. **Kanıtlanabilir mutabakat:** journal + Denetçi + Mutabakat sekmesi + FIX ham kaydı → "LP ne dedi, biz ne yaptık" her emir için.
3. **Fiyatlama laboratuvarı:** bantlar, skew, zamanlı markup, formül dili, what-if geri oynatma, sandbox.
4. **Operasyon disiplini:** mavi/yeşil, sağlık kapıları, yedek tatbikatı, alarm kural editörü, davranış analizi.
5. **Hız:** Rust motor, 10k emir/dk sınavı, LMAX 34 ms MD gecikmesi ölçülü.

Bilinen boşluklar (dürüst): MT5 eklentisinin kendisi (plan var, `docs/11`), çok-bölgeli yatay ölçek, Parquet/DuckDB, LP karşılaştırma grafiği, NDF/opsiyon ürünleri, regülasyon raporlama paketleri (MiFIR/EMIR), müşteri robot altyapısı kademe 2–3.

---

## 4. Lisanslama yelpazesi

İlke: **çekirdek + modül + ölçüm**. Her paket bir çekirdek seti, dahil modüller, ölçüm sınırları (hesap, LP, aylık lot, kurum), hizmet seviyesi ve sözleşme süresi içerir. Aşağıdaki fiyatlar **liste fiyatıdır** (USD, vergiler hariç); yüksek referans bantları esas alınmıştır, pazarlık payı liste üzerinden ‑%20'ye kadar.

| Kademe | Ad | Model | Dahil modüller | Ölçüm sınırı | Liste fiyatı | Süre |
|---|---|---|---|---|---|---|
| L1 | **Nexus Execute** | Aylık, barındırılan (bizim bulut), A/B-book taşıma + icra politikası | NX-CORE, NX-RISK (kural+bantlar+spread), NX-BRIDGE (tek LP), NX-AUDIT, NX-GATE (REST/WS), NX-WATCH (yerleşik) | 500 hesap, 1 LP, 2.000 lot/ay dahil; aşan lot 2,5 $/lot | **4.900 $/ay** + kurulum 5.000 $ | 12 ay, aylık fatura |
| L2 | **Nexus Bridge Pro** | Aylık, barındırılan; çoklu LP ve tam risk | L1 + NX-RISK tam (skew, VaR/TWAP, para birimi, patlama, last look, algo), NX-BRIDGE çoklu LP, NX-DATA, NX-WATCH kural editörü | 2.000 hesap, 3 LP, 10.000 lot/ay; aşan 1,5 $/lot | **9.900 $/ay** + kurulum 10.000 $ | 12 ay |
| L3 | **Nexus Prime** | Aylık/yıllık, barındırılan; tam platform (terminal + konsol + kimlik + copy + MT5 köprü) | L2 + NX-TERM (web/PWA, masaüstü, mobil kabuk), NX-CONSOLE, NX-ID, NX-COPY, NX-MT5 (1 kurum) | 10.000 hesap, 5 LP, 50.000 lot/ay; aşan 1 $/lot; ek kurum 1.500 $/ay | **19.900 $/ay** veya **199.000 $/yıl** + kurulum 25.000 $ | 12/24 ay |
| L4 | **Nexus White Label** | Yıllık; müşterinin markası, alan adı, renkleri; ayrı tenant; 7/24 SLA; özel yayın penceresi | L3 tamamı + tenant markalama, ayrı web sitesi/terminal alan adı, özel e-posta/ekstre şablonları, API anahtarı tahsisi, özel raporlar | 50.000 hesap, 10 LP, 250.000 lot/ay; aşan 0,6 $/lot | **24.900 $/ay** veya **249.000 $/yıl** + kurulum 35.000 $ | 24 ay |
| L5 | **Nexus Enterprise (kendi altyapısı)** | Yıllık; müşterinin bulutunda/veri merkezinde kurulum, iki bölge, mavi/yeşil, DR tatbikatı; kaynak erişimi (salt-okunur, escrow) | L4 + NX-OPS devri, dağıtım betikleri, izleme, felaket kurtarma; kaynak kodu escrow; güvenlik incelemesi raporu | sınırsız hesap; 15 LP; 1M lot/ay; aşan 0,3 $/lot | **480.000 $/yıl** + kurulum 120.000 $ | 36 ay |
| L6 | **Nexus Sovereign (marka ve isim hakkı devri)** | Çok yıllık/kalıcı; tam kullanım serbestisi, bölgesel münhasırlık, kendi markasıyla alt-lisanslama hakkı, isim hakkı devri seçeneği | Tüm modüller + kaynak kodu (değiştirme hakkı), yeni modüllerde ilk erişim, eğitim/teknoloji transferi, 12 ay yerinde mühendis | sınırsız; münhasırlık: ülke/bölge başına | **1.200.000 $/yıl (min 3 yıl)** veya **kalıcı lisans 3.500.000 $** + yıllık bakım %18 | 3+ yıl / kalıcı |

Ek kurallar:
- **Hacim katmanı:** aylık lot aşımı faturaya eklenir; yıllık ödemede %10 indirim; 24 ay sözleşmede kurulum ücreti yarıya iner.
- **SLA:** L1–L2 iş günü 9/5, %99,5; L3 7/24 kritik, %99,9; L4+ %99,95, 15 dk müdahale, aylık rapor, kredi: her 0,1 puan için fatura %5.
- **Güncellemeler:** sürüm güncellemeleri tüm kademelerde dahil; **yeni modüller** §7'ye göre ayrı lisanslanır (L6 ilk 12 ay ücretsiz dahil).
- **Veri:** müşteri verisi müşterinindir; L4+ ayrı tenant/veri tabanı; L5+ veri tamamen müşteride.

---

## 5. Ek modül ve hizmet fiyat listesi (à la carte)

| Kalem | Birim | Liste fiyatı | Not |
|---|---|---|---|
| Ek LP bağlantısı (FIX 4.2/4.4) | LP/ay | 1.500 $ | Yeni venue uyarlaması (yeni spec) + 15.000 $ tek sefer |
| NX-AUDIT (Denetçi) L1 dışı ayrı | ay | 2.500 $ | L2+ dahil |
| NX-DATA (ambar + analitik) | ay | 3.500 $ | L2+ dahil; saklama uzatma 90 gün +1.000 $ |
| NX-WATCH kural editörü + Telegram | ay | 900 $ | L2+ dahil |
| NX-COPY | ay | 2.000 $ | L3+ dahil |
| NX-MT5 ek kurum (institution) | kurum/ay | 1.500 $ | hacim 0,5 $/lot |
| TradingView webhook / API erişimi (müşteri başına) | müşteri/ay | 15 $ | veya pakete dahil (L3+) |
| Algoritmik fiyatlama (formül modülü) | ay | 1.500 $ | L2+ dahil |
| Dahili eşleştirme (netleştirme) | ay | 2.000 $ | L2+ dahil; spread kazancı müşteride |
| Ek tenant (çok markalı broker) | tenant/ay | 3.500 $ | L4+ |
| Masaüstü (Tauri) / mobil (Expo) imzalı yayın | platform/yıl | 12.000 $ | mağaza hesapları müşterinin |
| Profesyonel hizmet (mühendis) | gün | 1.800 $ | uyarlama, entegrasyon, eğitim |
| Yerinde kurulum (L5) | hafta | 15.000 $ | seyahat hariç |
| Kaynak kodu escrow | yıl | 6.000 $ | üçüncü taraf escrow ajanı |
| Güvenlik incelemesi / pen-test raporu | yıl | 20.000 $ | dış denetçi ile |

---

## 6. Fiyatlama dayanağı — depo alanlarına atıf

Fiyat, her modülün **kod hacmi, kanıtlanmış kapasite ve rakip bandı** üçlüsünden türetilmiştir:

| Modül | Depo alanı | Satır | Kanıt (sınama/canlı) | Rakip bandı ($/ay) | Liste payı |
|---|---|---|---|---|---|
| NX-CORE | `crates/oms`, `crates/ledger`, `crates/money`, `crates/domain`, `services/core-engine` | ~35k | 92 oms testi, replay eşitliği, 10k emir/dk yük sınavı | MT5 çekirdeği (lisans 5–10k) | çekirdek |
| NX-RISK | `crates/risk`, `oms` hedge/kural | ~5k | 21 risk testi + oms kural/hedge testleri; canlı demo-retail | 5–25k | 4–9k |
| NX-BRIDGE | `crates/fix-codec`, `fix-session`, `services/fix-gateway`, `lp_agg.rs`, `lp_fix.rs` | ~10k | LMAX canlı 2 oturum, codec 14 + session 17 + gateway 23 test, e2e/TLS | 1,5–15k | 4–8k |
| NX-AUDIT | `fix-gateway/src/bin/denetci.rs`, `audit.rs`, `fixlog.rs` | ~2k | canlı 2 sn döngü, −1 XAU düzeltmesi (9 Eki), "0 olay" kapısı | yok (benzersiz) | 1,5–3k |
| NX-GATE | `services/client-gateway`, `crates/client-proto` | ~9k | REST/WS e2e, hız limiti, webhook testi | pakete dahil | — |
| NX-ID | `services/identity` | ~5,7k | TOTP/passkey/API anahtarı akışları canlı | 1–2k | dahil |
| NX-TERM | `apps/terminal`, `desktop`, `mobile` | ~17k | PWA canlı (trade.fxvps.ai), e2e | 2–6k | 3–6k |
| NX-CONSOLE | `apps/backoffice`, `core-engine/src/admin` | ~13k + admin | canlı (console.fxvps.ai), RBAC/MFA | MT5 Manager | dahil |
| NX-MT5 | `bridge.rs`, `bridge_admin.rs` | ~2k | köprü WS canlı, kurum anahtarları | 1–3k/kurum | 1,5k/kurum |
| NX-DATA | `ticks.rs`, `volcalc.rs`, analitik görünümleri | ~1,5k | ambar canlı (9 Eki'den), σ saatlik | 2–5k | 3,5k |
| NX-WATCH | `alerts.rs`, `activity.rs` | ~1,5k | canlı kurallar (12 Eki) | dahil/Pro | 0,9k |
| NX-OPS | `deploy/lmax-demo`, `lp-simulator`, `loadgen`, docs 04/05/09 | ~3k + betikler | mavi/yeşil günde 10+ yayın (9–11 Eki), yedek tatbikatı | işletme | kurulum |

Toplam ~110k satır Rust/TypeScript, 2 canlı LP oturumu (LMAX), 4 canlı alan adı, günlük yayın disiplini. Referans: oneZero Hub + MT5 white label + analitik + kimlik ayrı ayrı alındığında 25–45k $/ay'a varır; **Nexus Prime (19.900 $/ay)** bu sepetin tek çatıda, denetçili halidir.

---

## 7. Genişleyen yelpaze — yeni modül ve koşul güncelleme usulü

1. **Modül kataloğu versiyonlu:** bu belge `docs/16` olarak sürümlenir (1.0 → 1.1 …); her yeni modül kod (NX-XXX), depo alanı, kapasite ve liste fiyatıyla eklenir.
2. **Ek lisans sözleşmesi (Addendum):** mevcut lisans sahibine yeni modül, ana sözleşmeye ek protokol ile verilir; fiyat liste veya kademe indirimi (L3 %10, L4 %20, L5 %30, L6 ilk 12 ay dahil).
3. **Kullanım şartları güncellemesi:** 60 gün önceden bildirim; fiyat artışı yıllık en çok TÜFE+%5 veya %8 (hangisi düşükse); müşteri 30 gün içinde fesih hakkı (kalan dönem iadesi yok, hizmet dönem sonuna kadar sürer).
4. **Modül devre dışı bırakma:** müşteri bir modülü sözleşme yenilemesinde çıkarabilir; veri dışa aktarımı 90 gün sağlanır.
5. **Yol haritası modülleri (aday):** NX-ROBOT (müşteri robot altyapısı: terminal içi strateji betiği + backtest; sunucuda barındırma), NX-REG (MiFIR/EMIR/regülatör raporları), NX-PRIME (prime-of-prime: alt-broker'lara LP hizmeti, NX-MT5 + NX-BRIDGE), NX-CRYPTO (kripto LP/ödeme), NX-LAKE (Parquet/DuckDB ambar, LP karşılaştırma grafiği), NX-ML (davranış/markout tahmin modelleri).

---

## 8. Sözleşme iskeleti (hukuk ekibine devredilecek)

- **Ana Lisans Sözleşmesi (MLA):** taraflar, tanımlar (modül, tenant, hesap, lot, LP), lisans kapsamı ve kademe, ölçüm ve faturalama, SLA eki, destek eki, güncelleme/yeni modül eki (§7), fikri mülkiyet (kaynak kodu mülkiyeti bizde; L6'da devir/alt-lisans hakkı), gizlilik, veri işleme (DPA, KVKK/GDPR), sorumluluk sınırı (12 aylık ücret), mücbir sebep, fesih ve geçiş (veri dışa aktarımı 90 gün), uygulanacak hukuk/tahkim (DIFC-LCIA veya ICC).
- **Ekler:** A) Modül kataloğu (bu belge §1–2), B) Fiyat listesi (§4–5), C) SLA, D) Destek ve yayın takvimi, E) Güvenlik ve DR (docs/06, 09), F) Kabul testleri (yük sınavı, Denetçi 0 olay, e2e), G) Escrow (L5+), H) Marka kullanım kılavuzu (L4+), I) Münhasırlık ve isim hakkı devri (L6).
- **Uyumluluk notu:** yazılım lisansı mali hizmet lisansı değildir; müşteri kendi regülasyonundan sorumludur (sözleşmede açık beyan).

---

## 9. Lisanslanabilirlik hazırlık listesi (teknik)

Depoda bugün var olanlar ✅ ve lisanslı dağıtım için eksikler 🔧:

| Konu | Durum | Depo/plan |
|---|---|---|
| Tenant/markalama (alan adı, renk, logo, destek e-postası) | ✅ | `admin/store.rs` TenantRec, console Ayarlar → Tenants |
| Modül bayrakları (lisansa göre özellik açma/kapama) | 🔧 | `AdminConfig`/`SettingsRec` içine `license: {tier, modules[], limits{accounts, lps, lots_month}, expires}`; imzalı lisans dosyası (Ed25519) → console ve API kapıları |
| Ölçüm (aylık lot, hesap sayısı, LP sayısı) ve fatura dışa aktarımı | 🔧 | `admin/views.rs` revenue/reports üzerine `GET /v1/license/usage` |
| İmaj dağıtımı (ghcr) + sürüm/imza | ✅/🔧 | CI imajları var; imaj imzalama (cosign) ve sürüm notları eklenmeli |
| Kurulum paketi (tek kutu compose + sağlık kapıları) | ✅ | `deploy/lmax-demo` (`docs/04`) — müşteri için `deploy/template/` türetilmeli |
| Çok-bölge / yatay ölçek | 🔧 | `docs/03` yol haritası (Etap 10 ölçek tatbikatı var, çok-bölge yok) |
| Kaynak kodu escrow | 🔧 | üçüncü taraf ajanı; depo etiketleri sürümlenmeli |
| Dokümantasyon (kurulum, operasyon, API, güvenlik) | ✅ | `docs/02–09`, `api.md`, `openapi.yaml`, `README.md` |
| Kabul testleri | ✅ | `cargo test`, e2e (client-gateway/fix-gateway), yük sınavı, Denetçi kapısı |

Önerilen ilk teknik iş: **imzalı lisans dosyası + modül bayrakları + kullanım ölçümü** (NX-LICENSE, ~1 hafta) — bu olmadan L1–L3 kademeleri sözleşmeyle değil yalnızca güvenle ayrışır.

---

## 10. Örnek teklifler

- **Yeni başlayan broker (500 hesap, tek LP, A/B kural):** Nexus Execute 4.900 $/ay + 5.000 $ kurulum; 3. ayda 3.000 lot → +2.500 $. Yıl: ~70k $.
- **Büyüyen broker (3 LP, risk masası, analitik):** Nexus Bridge Pro 9.900 $/ay + Denetçi dahil; MT5 köprüsü 1 kurum +1.500 $. Yıl: ~150k $.
- **Marka sahibi broker (kendi terminali/konsolu, 2 tenant):** Nexus White Label 249.000 $/yıl + ek tenant 42.000 $ + kurulum 35.000 $. Yıl 1: ~326k $.
- **Bölgesel teknoloji sağlayıcısı (alt-broker'lara satış):** Nexus Sovereign 1,2M $/yıl × 3 yıl, bölgesel münhasırlık (ör. MENA), alt-lisanslama hakkı; veya kalıcı 3,5M $ + %18 bakım.

Toplam adreslenebilir gelir (illüstratif, 24 ay): 10 Execute + 6 Bridge Pro + 3 Prime + 2 White Label + 1 Enterprise ≈ **2,9M $/yıl** yinelenen; tek Sovereign anlaşması bunun üstüne.
