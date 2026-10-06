# Bridge seviyesi yol haritası (PrimeXM / FXCubic yelpazesi)

Kurucu kararı (6 Ekim 2026): konsol ve çekirdek, en üst düzey bir bridge
firmasının tab/özellik yelpazesiyle donatılacak; "bizde olmayan kalmasın".
Bu belge o yelpazeyi **bugünkü duruma göre** sıralar. Her etap tek PR'lık
parçalara bölünür, her parça canlıya iner, sonra sıradaki alınır.

Durum işaretleri: ✅ var · 🟡 kısmen · ⬜ yok

## 0. Bugün elimizde olan

| Alan | Durum | Not |
|---|---|---|
| A-book STP (LMAX FIX, IOC market + limit) | ✅ | tek LP, omnibus |
| B-book (iç karşı taraf) | ✅ | grup bazında `routing` |
| Grup: kaldıraç, margin/stop-out, markup (puan), fiyat toleransı, ESMA, NBP | ✅ | |
| Sembol: sözleşme, lot adımları, swap, komisyon | ✅ | swap **uygulanmıyor** (rapor 0) |
| Derinlik (5 seviye, markup'lı) terminale | ✅ | #82 |
| Raporlar: kapanan işlemler, ekstre, LP icraları, gelir (markup/B-book/komisyon) | ✅ | |
| Dashboard: A-book P&L, LP durumu, maruziyet | 🟡 | |
| Kayma raporu (müşteri / LP / gecikme / deneme) | ⬜ | **Etap 1** |
| Kısmi dolum politikası | 🟡 | PartiallyFilled var; politika (kabul/iptal kalan/yeniden dene) yok |
| Çoklu LP, agregasyon, VWAP/best-price yönlendirme | ⬜ | tek LP |
| Hibrit (A/B karışık, hacim/müşteri/sembol bazlı) | ⬜ | |
| Kural motoru (müşteri/sembol/hacim → defter, markup, slippage) | ⬜ | |
| Swap/rollover uygulaması | ⬜ | |

## Etaplar (sırayla)

### Etap 1 — Yürütme kalitesi raporu ✅ (#83)
`/v1/reports/execution` + konsolda "Yürütme" sekmesi. Her müşteri emri için:
istenen fiyat, dolum fiyatı, **müşteri kayması (puan, +=aleyhine)**, LP ortalama
fiyatı, **LP'ye karşı yakalanan fark (puan)**, LP gecikmesi (gönderim→ilk
dolum, ms), LP dolum adedi (kısmi dolum izi), yeniden kurma (re-arm) bayrağı,
defter. Sembol bazında özet: emir adedi, dolum oranı, kısmi oranı, ortalama ve
p95 kayma, fiyat iyileşme payı, ortalama gecikme, ortalama yakalama. CSV.

### Etap 2 — LP kaymasının gerçek ölçümü ✅ (#90)
LP emri gönderilirken **o andaki LP kotasyonu** `LpOrder`'a yazılır
(`sent_bid/sent_ask`); rapor "LP'nin yaşattığı kayma"yı istenen değil, gönderim
anındaki LP fiyatına göre verir. Deneme sayacı (`attempts`) gerçek sayaca döner
(re-arm bayrağı yerine). Gecikme p50/p95/p99.

### Etap 3 — Kısmi dolum politikası (grup bazında) ✅ (#91)
*kalanı iptal et* (varsayılan), *kalanı yeniden dene (N LP emri)*, *all-or-none
(LP'ye FOK)*. "Kalanı B-book'a al" bilinçli dışarıda: A-book pozisyonu omnibus
hedge'siz kalır, dolum başına defter izi gerekir → Etap 7'de.

### Etap 4 — Kazanç kapıları: markup/slippage/komisyon kuralları ✅ (#93; hacim kademeli komisyon ve önizleme sonraya)
Bugün tek `markup_points`. Eklenecek: alış/satış ayrı markup, sembol ve
sembol-grubu bazlı ek markup, **asimetrik slippage** (müşteri lehine kayma
yansıtılmaz / kısmen yansıtılır), maksimum müşteri kayması (üstü red / fiyat
iyileştirme), komisyon (lot/işlem/notional bps, min-maks), hacim kademeli
komisyon. Konsolda "Kazanç kalemleri" sihirbazı: her kalem tek kart, önizleme
("bu ayarla dünkü akışta gelir X olurdu" — Etap 1 verisinden).

### Etap 5 — Kural motoru (hibrit yönlendirme) ✅ (#106)
Sıralı kurallar: *müşteri / grup / sembol / hacim / emir tipi / saat* →
*defter (A/B/yüzde A)*, *markup profili*, *slippage profili*, *LP*. İlk eşleşen
kazanır. Konsolda sürükle-bırak liste + kuru koşum ("son 24 saatte hangi emir
hangi kurala düşerdi").

### Etap 6 — Çoklu LP ve agregasyon ✅ (LP simülatörü ikinci LP; simülatör kalıcı)
İkinci FIX oturumu (önce LMAX ikinci hesap / LP simülatörü). Agregasyon
defteri; yönlendirme: *best price*, *VWAP (hacme göre)*, *öncelikli LP*,
*round-robin*; LP bazında min/max emir, sembol haritası, kesme (kill switch).
Rapor: LP bazında dolum oranı, kayma, red, gecikme karşılaştırması.

Yapılan: `FIX_LP_FILES` ile LP başına bir fix-gateway; `lp_agg::Aggregator`
(LP bazında defter → motor birleşik en iyi alış/satış'ı günlükler, müşteri
birleşik derinliği görür); `AggLpRouter` kip + politika ile LP seçer, kapalı
kanalda sıradakine düşer, seçimi `Command::LpRouted` ile günlükler;
LP başına kill switch / öncelik / min-maks lot / sembol listesi; sapma
koruması (öncelik-1 LP'den N puan uzak LP yok sayılır, çapraz defterde
referansa dönülür). Konsol: LP sayfası → Agregasyon kartı + LP performansı
(`/v1/lp/aggregation`, `/v1/reports/lp`), LP icralarında LP sütunu.
Sonraki: simülatörün gerçek fiyatı izlemesi (follow) ve konsoldan senaryo
tetikleme (şok, ret, gecikme) — simülatör test aracı olarak kalıcı.

### Etap 7 — B-book ileri yönetim ✅
Maruziyet limitleri (sembol/müşteri/toplam) aşılınca otomatik hedge (A'ya
geçiş veya LP'de kısmi hedge), müşteri karlılık profili (toxic flow skoru:
kısa tutma süresi, haber anı, kayma kazanımı), profil → kural motoru girdisi.

Yapılan: `HedgePolicy` (motor günlüğünde, `Command::SetHedge`): sembol /
toplam / müşteri-başı net B-book limitleri; kip **switch_to_a_book** (limit
üstünde risk artırıcı yeni akış A-book'a, emir `hedge:limit` etiketli) veya
**hedge_excess** (akış B-book'ta kalır, fazlalık omnibus hedge defterinde
LP'ye gönderilir, `release_pct` altına inince çözülür; hedge K/Z broker
defterine yazılır; değişmez: omnibus = A-book net + hedge). `FlowStats`
müşteri profili (tutma süresi, kazanma, yakalanan fiyat iyileşmesi) →
**toksisite 0–100**; kural motoru `minToxicity/maxToxicity` süzgeci.
Konsol: Risk → hedge kartı + maruziyet tablosunda Hedge/Limit; Raporlar →
"Müşteri akışı"; müşteri kartında toksisite rozeti; kural düzenleyicide
toksisite alanları. Haber-anı ölçütü takvim kaynağı gelince eklenecek.

### Etap 8 — Swap/rollover ve ücret motoru ✅
Günlük swap uygulaması (çarşamba 3×), sembol bazında; ekstre ve gelir raporuna
"swap" kalemi gerçek olur.

Yapılan: `SwapConfig` (saat UTC, hafta sonu atla, açık/kapalı; `Command::SetSwapConfig`),
stack içinde zamanlayıcı; motor rollover'ı UTC günü başına **bir kez** uygular
(yeniden başlatmaya dayanıklı), sembolün `triple_swap_day`'inde 3 gün, grup
`swap_multiplier_pct` ile ölçekler, `SwapMode::{Money, Points}`; swap pozisyonda
birikir, kapanışta kapanan parçayla deal'e geçer → pozisyon/işlem/ekstre/gelir
raporlarında gerçek swap. Konsol: Ayarlar → Swap kartı (+ "şimdi koş"),
sembolde swap tipi/üçlü gün, grupta swap çarpanı. Terminal pozisyon swap alanı
proto genişlemesi bekliyor.

### Etap 9 — Dashboard ve rapor genişliği ✅
Saatlik/günlük seriler (hacim, gelir kalemleri, kayma, gecikme), LP sağlık
paneli (gerçek seq/latency), uyarılar (dolum oranı düşüşü, gecikme sıçraması,
LP kopması, maruziyet limiti).

Yapılan: dashboard kovalarına kayma (ort. puan), LP p95 gecikme, dolum sayısı ve
swap eklendi + "yürütme kalitesi" grafiği; LP oturum tablosunda gerçek
MsgSeqNum ve son mesaj zamanı (`GatewayEvent::SessionStats`); **uyarı
motoru** (`admin/alerts.rs`, 15 sn): LP oturumu düşük (>60 sn), dolum oranı
<%90 (1 saat, ≥10 emir), p95 gecikme sıçraması (>500 ms ve 3× önceki saat),
B-book maruziyet limiti aşımı, sapma korumasının dışladığı LP, stop-out'taki
hesaplar. Her yeni uyarı denetim defterine yazılır, `CORE_ALERT_WEBHOOK_URL`
varsa JSON POST edilir; konsolda üst çubukta zil + dashboard uyarı kartı +
onaylama. Eşikler şimdilik sabit (ayar sayfası Etap 13).

---

## Sonraki beş etap — Claude'un önerisi (Etap 10–14)

Kurucunun 1–9 etabı "bir bridge'de olması gerekenler"di; aşağıdakiler gerçek
para ve gerçek müşteri gelmeden önce **operasyonun dayanıklılığını** ve
**ticari yüzeyi** tamamlar. Sıra, risk ↓ ve gelir ↑ etkisine göre.

### Etap 10 — Felaket kurtarma ve sıfır kesintili dağıtım ✅ (mavi/yeşil hariç → Etap 14)
Bugün tek CT, tek disk, tek süreç. Hedefler: (a) günlük motor journal +
admin store + candles **şifreli offsite yedeği** ve **otomatik geri yükleme
provası** (yedekten boş CT'ye kalk, snapshot digest'i karşılaştır);
(b) **mavi/yeşil trading süreci**: yeni imaj journal'ı replay edip "hazır"
dedikten sonra FIX oturumları ve WS bağlantıları devredilir (müşteri
~45 sn kopma yaşamaz); (c) **chaos sınavı**: LP kesintisi, disk dolması,
saat sapması, çift süreç senaryoları CI'da koşar; (d) journal boyutu
büyüdükçe **snapshot + sıkıştırılmış arşiv** (bugün 235 MB, lineer büyüyor).

Yapılan: `core-engine verify|compact`, `yedek.sh` (şifreli + günlük geri yükleme
provası + durum dosyası → uyarı motoru), `geri-yukle.sh`, `dagit.sh` (replay
kapısı + geri dönüş etiketi + sağlık kontrolü + otomatik geri alma),
başlangıçta journal sıkıştırma. Ayrıntı: `docs/09-felaket-kurtarma.md`.

### Etap 11 — Uyum, raporlama ve denetim izi ✅ (e-posta teslimi Etap 12 mailer'a bağlı)
Lisans alındığında ilk sorulanlar: (a) **müşteri ekstresi PDF/e-posta**
(günlük/aylık, imzalı), (b) **MiFIR/EMIR benzeri işlem raporu dışa aktarımı**
(CSV/XML şablonu, LEI alanları), (c) **best execution raporu** (yürütme
kalitesi verimiz zaten var → RTS 27/28 formatına döküm), (d) **değiştirilemez
denetim**: audit zinciri hash-zincirli + günlük kök hash'i dış kayda (örn.
Git tag / zaman damgası servisi), (e) **negatif bakiye koruması ve stop-out
olaylarının müşteriye bildirimi** (e-posta/terminal).

Yapılan: **işlem raporu** `GET /v1/reports/transactions?from&to` (RTS 22 alan
alt kümesi: LEI'ler, kapasite DEAL/MTCH, mekân XOFF, LP, birim miktar, nominal)
+ CSV; **best execution** `GET /v1/reports/best-execution` (mekân × varlık
sınıfı: hacim payı, dolum, kayma, fiyat iyileşmesi, gecikme) + CSV; **müşteri
LEI** (profil, kartta düzenlenir) ve **broker LEI** (Ayarlar); **hash-zincirli
denetim** (`AuditRec.prev_hash/hash`, `GET /v1/audit/chain`, denetim sayfasında
rozet) + gece yedeğinin konteyner dışında sakladığı **append-only çapa**
(admin.jsonl uzunluk+hash; önek değişirse yedek durur → uyarı); **yazdırılabilir
ekstre** `/clients/statement/?login=&from=&to=` (tarayıcı PDF). Kalan: e-posta
ile ekstre/bildirim (mailer ile birlikte Etap 12).

### Etap 12 — Müşteri yaşam döngüsü ve ödeme ✅ (demo→gerçek dönüşüm ve KYC sağlayıcı entegrasyonu sonraya)
Konsolda hesap var ama müşteri hunisi yok: (a) **KYC akışı** (belge yükleme,
durum makinesi, onay/ret nedeni, sağlayıcı entegrasyon noktası),
(b) **para yatırma/çekme talepleri** (kripto USDT TRC-20 — mt5forexvps'teki
eşleyici burada yeniden kullanılır — + banka havalesi talimatı), çift onay
mevcut 4-göz mekanizmasına bağlanır, (c) **IB/partner ağacı**: alt hesaplar
zaten var → komisyon paylaşımı, IB raporu, IB portalı (salt-okunur),
(d) **demo→gerçek dönüşüm**: demo hesap süresi, bakiye sıfırlama, "gerçek
hesaba geç" düğmesi.

Yapılan: **müşteri self-servis API** (`/api/client/*` terminal origin'inde →
geçit → admin `/v1/client/*`, müşteri jetonu `accounts` iddiasıyla doğrulanır):
`me`, para yatırma/çekme talebi (USDT TRC-20 / banka, asgari tutar, bakiye
kontrolü), KYC belge yükleme (JPEG/PNG/WebP/PDF ≤6 MB, sha256, dosya
`core-data/kyc/<login>/`, ilk yüklemede KYC → pending). Terminalde **"Para
işlemleri"** diyaloğu (talimatlar, talep formu, geçmiş; doğrulama sekmesi).
Konsolda Onaylar sayfasında **müşteri para talepleri** (onayla → bakiye işlemi,
eşik üstünde çift onay; reddet; çekmeyi "ödendi" işaretle), kartta KYC
belgeleri (aç) ve not, Ayarlar'da yatırma talimatları. **IB**: hesaba IB payı ve
IB bağlantısı, `GET /v1/reports/ib` + Raporlar sekmesi + CSV.

### Etap 13 — Operasyon ayarları ve otomasyon
Sabitlerin hepsi ayara dönsün: (a) **uyarı eşikleri ve kanalları** (Telegram
bot, e-posta, webhook; sessiz saatler), (b) **piyasa saatleri / tatil
takvimi**: sembol seansları dışında emir reddi, haftasonu swap ve rollover
takvime bağlanır, haber takvimi bağlanınca toksisite "haber anı" ölçütü
tamamlanır, (c) **LP simülatörü senaryo paneli**: konsoldan fiyat şoku, ret
oranı, gecikme enjeksiyonu (simülatör zaten kalıcı), (d) **kural motoru
sürümleme**: kural setleri etiketli, geri alma, "dry-run'ı geçmiş 7 güne
uygula" karşılaştırması, (e) **günlük operasyon raporu** (Telegram'a özet:
hacim, gelir, uyarılar, LP sağlığı).

### Etap 14 — Performans, ölçek ve çoklu kiracı
Birden fazla marka/broker aynı çekirdekte: (a) **kiracı (tenant) ayrımı**:
gruplar ve hesaplar kiracıya bağlı, konsol rolü kiracı kapsamlı, ayrı
Cloudflare hostname'leri, (b) **yük ve gecikme bütçesi**: 1k eşzamanlı WS,
10k emir/dk hedefi; `loadgen` CI'da; p99 emir yolu < 5 ms motor içi,
(c) **NATS ile süreç ayrımı** (belgede "follow-up" diye duran): fix-gateway,
core-engine, client-gateway ayrı konteyner, biri düşünce diğerleri yaşar,
(d) **okuma kopyası**: raporlar/dashboard sorguları motoru kilitlemesin
(snapshot'tan servis), (e) **FIX API müşterilere** (kurumsal müşteri kendi
FIX oturumuyla bağlanır; fix-session crate'i zaten acceptor rolünü biliyor).

## Her etabın teslim kapısı
tsc/lint/test yeşil → PR → CI → merge → CT 970 → canlıda ölçüm (gerçek LMAX
demo akışıyla) → bu belgede ✅.
