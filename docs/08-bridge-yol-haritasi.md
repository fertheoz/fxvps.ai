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

### Etap 1 — Yürütme kalitesi raporu  ← ŞİMDİ
`/v1/reports/execution` + konsolda "Yürütme" sekmesi. Her müşteri emri için:
istenen fiyat, dolum fiyatı, **müşteri kayması (puan, +=aleyhine)**, LP ortalama
fiyatı, **LP'ye karşı yakalanan fark (puan)**, LP gecikmesi (gönderim→ilk
dolum, ms), LP dolum adedi (kısmi dolum izi), yeniden kurma (re-arm) bayrağı,
defter. Sembol bazında özet: emir adedi, dolum oranı, kısmi oranı, ortalama ve
p95 kayma, fiyat iyileşme payı, ortalama gecikme, ortalama yakalama. CSV.

### Etap 2 — LP kaymasının gerçek ölçümü
LP emri gönderilirken **o andaki LP kotasyonu** `LpOrder`'a yazılır
(`sent_bid/sent_ask`); rapor "LP'nin yaşattığı kayma"yı istenen değil, gönderim
anındaki LP fiyatına göre verir. Deneme sayacı (`attempts`) gerçek sayaca döner
(re-arm bayrağı yerine). Gecikme p50/p95/p99.

### Etap 3 — Kısmi dolum ve reddetme politikası (grup + sembol bazında)
Seçenekler: *kalanı iptal et* (bugünkü), *kalanı yeniden dene (N deneme, M ms)*,
*kalanı B-book'a al*, *tamamı reddet (all-or-none)*. Konsolda grup formunda
basit radyo + iki sayı. Rapor Etap 1'e "politika sonucu" sütunu ekler.

### Etap 4 — Kazanç kapıları: markup/slippage/komisyon kuralları
Bugün tek `markup_points`. Eklenecek: alış/satış ayrı markup, sembol ve
sembol-grubu bazlı ek markup, **asimetrik slippage** (müşteri lehine kayma
yansıtılmaz / kısmen yansıtılır), maksimum müşteri kayması (üstü red / fiyat
iyileştirme), komisyon (lot/işlem/notional bps, min-maks), hacim kademeli
komisyon. Konsolda "Kazanç kalemleri" sihirbazı: her kalem tek kart, önizleme
("bu ayarla dünkü akışta gelir X olurdu" — Etap 1 verisinden).

### Etap 5 — Kural motoru (hibrit yönlendirme)
Sıralı kurallar: *müşteri / grup / sembol / hacim / emir tipi / saat* →
*defter (A/B/yüzde A)*, *markup profili*, *slippage profili*, *LP*. İlk eşleşen
kazanır. Konsolda sürükle-bırak liste + kuru koşum ("son 24 saatte hangi emir
hangi kurala düşerdi").

### Etap 6 — Çoklu LP ve agregasyon
İkinci FIX oturumu (önce LMAX ikinci hesap / LP simülatörü). Agregasyon
defteri; yönlendirme: *best price*, *VWAP (hacme göre)*, *öncelikli LP*,
*round-robin*; LP bazında min/max emir, sembol haritası, kesme (kill switch).
Rapor: LP bazında dolum oranı, kayma, red, gecikme karşılaştırması.

### Etap 7 — B-book ileri yönetim
Maruziyet limitleri (sembol/müşteri/toplam) aşılınca otomatik hedge (A'ya
geçiş veya LP'de kısmi hedge), müşteri karlılık profili (toxic flow skoru:
kısa tutma süresi, haber anı, kayma kazanımı), profil → kural motoru girdisi.

### Etap 8 — Swap/rollover ve ücret motoru
Günlük swap uygulaması (çarşamba 3×), sembol bazında; ekstre ve gelir raporuna
"swap" kalemi gerçek olur.

### Etap 9 — Dashboard ve rapor genişliği
Saatlik/günlük seriler (hacim, gelir kalemleri, kayma, gecikme), LP sağlık
paneli (gerçek seq/latency), uyarılar (dolum oranı düşüşü, gecikme sıçraması,
LP kopması, maruziyet limiti).

## Her etabın teslim kapısı
tsc/lint/test yeşil → PR → CI → merge → CT 970 → canlıda ölçüm (gerçek LMAX
demo akışıyla) → bu belgede ✅.
