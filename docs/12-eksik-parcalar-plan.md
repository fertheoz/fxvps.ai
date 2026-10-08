# 12 — Eksik Parçalar: Kesintisiz Geliştirme Planı

> **Tarih:** 2026-10-08. **Kaynak:** rakip matrisi (docs/01 §18) ve koddaki gerçek durum taraması.
> **Çalışma kipi:** her madde sırayla yapılır. Akış her maddede aynıdır: dal aç, kodla, sınav yaz, CI yeşil olunca birleştir, CT 970'te `./dagit.sh` ile yayınla, canlıda doğrula, bu dosyada durumu ✅ yap, sıradakine geç. Kurucudan onay beklenmez.
> **Kurucuya kalanlar** (MFA, sır girişi, dış hesap): beklenmez. Bu işler §Kurucu kuyruğu'na yazılır ve iş, o adımı atlayan güvenli bir varsayılanla sürer.

## Değişmez kurallar
- Sırlar (LMAX parolası, jetonlar, anahtarlar) asla yazdırılmaz, loga düşmez, forma girilmez.
- Canlı para yolunu bozan değişiklik özellik bayrağı arkasında **kapalı** gelir. Demo grupta açılır, canlı grupta açmak kurucu kuyruğuna yazılır.
- Her madde: birim sınavı + mevcut sınav takımı yeşil + dağıtım sonrası sağlık OK. Denetçi 0 olay vermeli.
- Bir şey silinmez. Göçler yalnız eklemeli olur (yeni alan için `#[serde(default)]`).
- Bir madde 2 denemede de düşerse ⛔ notu düşülür ve sıradakine geçilir. Akış takılıp durmaz.
- Commit kimliği: fertheoz. PR gövdesi Claude Code imzasıyla biter.

---

## 1. Kademeli kaldıraç + swap-free hesap  ⬜
**Neden:** Körfez ve İslami pazarı için zorunlu. MT5'te "leverage tiers" ve swap-free standarttır.
- `risk::GroupConfig.leverage_tiers: Vec<{notional_usd, leverage}>`. Kaldıraç, sembol başına açık nominale göre kademeli düşer. Etkin kaldıraç = min(grup, hafta sonu, haber penceresi, kademe).
- `swap_free: bool` (grup ve hesap düzeyinde). Rollover'da swap uygulanmaz. İsteğe bağlı `admin_fee_per_lot_day` ve `swap_free_max_days` (sonrasında swap başlar).
- Konsol: grup formunda kademe tablosu + swap-free; hesap kartında rozet.
- Terminal: sembol bilgisinde etkin kaldıraç.
- **Kabul:** marj hesabında kademe geçiş sınavı; swap-free hesapta rollover 0; canlı demo grupta doğrulandı.

## 2. Kullanıcı API'si (REST + WS + kişisel anahtar)  ⬜
**Neden:** algo müşterileri ve cTrader Open API'nin karşılığı. Copy trading de bu temelin üstüne kurulur.
- identity: kişisel API anahtarı (yalnız SHA-256 özeti saklanır), kapsamlar `read` / `trade`, IP izin listesi, son kullanım zamanı, iptal.
- client-gateway: `/api/v1` REST (hesap, pozisyon, emir, geçmiş) + mevcut WS protokolü anahtarla. Anahtar başına hız sınırı.
- Terminal: Ayarlar → API anahtarları (oluştur, bir kez göster, iptal et). Belge: `docs/api.md` + OpenAPI.
- **Kabul:** anahtarla emir/iptal e2e; read anahtarı emir veremez; iptal edilen anahtar 401 alır; hız sınırı sınavı.

## 3. Copy trading / PAMM-MAM  ⬜
**Neden:** brokera satışta ilk sorulan özellik (cTrader Copy, MT5 Signals, B2Copy).
- Strateji sağlayıcı hesap → takipçi abonelikleri. Oranlar: eşit / bakiye oranlı / sabit çarpan. Takipçi başına en fazla kayıp (equity stop).
- Kopyalama motoru core-engine içinde çalışır: sağlayıcı dolumu takipçi emirlerine dağıtılır, hepsi tek LP emrinde toplanır (MAM).
- Ücretler: performans (HWM), yönetim, hacim. Ödeme dönemi sonunda ledger kaydı düşülür.
- Terminal: Strateji vitrini (getiri, maksimum düşüş, takipçi sayısı), abone ol / ayrıl. Konsol: stratejiler, ücret raporu, zorla ayırma.
- **Kabul:** 1 sağlayıcı + 3 takipçi sınavı (oranlar, kısmi kapatma, equity stop); HWM ücreti yalnız yeni zirvede; Denetçi net eşitliği korunuyor.

## 4. IB / affiliate komisyon motoru  ⬜
**Neden:** broker büyümesinin ana kanalı.
- IB ağacı (çok seviyeli). Kural: lot başı USD, spread payı % veya komisyon payı %. Seviye başına oran.
- Kayıt bağlantısı / referans kodu → hesap IB'ye bağlanır. Her deal'da tahakkuk olur, günlük toplanır, ödeme onay akışı (4 göz) ile yapılır.
- Konsol: IB'ler, ağaç, tahakkuk ve ödeme raporu. Partner girişine "IB" görünümü eklenir (kendi müşterileri, kazancı).
- **Kabul:** 2 seviyeli ağaçta tahakkuk sınavı; ödeme ledger'a düşer; IB yalnız kendi ağacını görür.

## 5. A/B-book kural motoru + toksisite skoru  ⬜
**Neden:** oneZero/Centroid'in ana satış noktası. Broker kârlılığını doğrudan etkiler.
- Hesap başına skor: kazanç oranı, ortalama tutma süresi, fiyat sonrası hareket (markout 1 sn / 5 sn / 60 sn), haber çevresinde işlem.
- Kurallar (öncelik sıralı): koşul (skor, grup, sembol, lot) → yönlendirme A / B / kısmi % hedge. Kural değişimi denetim kaydına düşer.
- B-book net pozisyonu için otomatik hedge eşiği (sembol başına net USD sınırı).
- Konsol: Kurallar sayfası, hesap skor tablosu, markout grafiği.
- **Kabul:** sentetik toksik hesap A-book'a geçer; hedge eşiği aşılınca LP emri çıkar; Denetçi uyumlu kalır.

## 6. Çoklu LP agregasyonu + en iyi icra raporu  ⬜
- Fiyat birleştirme (en iyi bid/ask, LP başına markup), LP sağlığına göre devre dışı bırakma, yönlendirme (en iyi fiyat / tercih sırası).
- En iyi icra raporu: LP ve sembol başına slipaj, dolum oranı, gecikme p50/p99, ret nedenleri. CSV/PDF.
- İkinci gerçek LP gelene kadar SIM ile sınanır.
- **Kabul:** LP düşünce otomatik geçiş sınavı; rapor konsolda.

## 7. White-label kurulumu  ⬜
- Tenant başına marka: ad, logo, renkler, alan adı (terminal + konsol), e-posta gönderen adı.
- Kurulum sihirbazı: tenant → grup → semboller → marka → yönetici davet.
- **Kabul:** `mishov` tenant'ı kendi markasıyla açılır; diğer tenant verisi görünmez.

## 8. Müşteri hesap ekstresi (PDF)  ⬜
- Günlük ve aylık ekstre: bakiye hareketi, işlemler, swap, komisyon, açık pozisyonlar. Terminalden indirilir ve e-postayla gönderilir (Resend).
- **Kabul:** PDF Türkçe/Arapça karakterle doğru; tutarlar ledger ile birebir.

## 9. Self-servis yatırma/çekme  ⬜
- Terminal: Yatır (USDT TRC-20 tek adres + sent-altı tutar eşleme; mt5forexvps'teki kanıtlanmış yöntem), Çek (talep → 4 göz → ödeme).
- Konsoldaki mevcut onay akışına bağlanır. Kart/banka sonradan eklenir.
- **Kabul:** sınama ağında eşleme sınavı; çekim, onaysız ledger'a düşmez.

## 10. Terminal eşitliği  ⬜
- Renko, Tick, Range grafikleri; sentiment (müşteri pozisyon dağılımı, anonim); ekonomik takvim pinleri grafikte; replay modu.
- **Kabul:** her biri için bileşen sınavı; canlı terminalde görünür.

## 11. Backtest (tarihsel mum + strateji API'si)  ⬜
- Madde 2'deki API'nin aynısıyla, candles verisi üzerinde sunucuda koşar ve rapor verir.
- **Kabul:** basit MA kesişimi stratejisinde deterministik sonuç.

---

## Kurucu kuyruğu (beklenmez, buraya yazılır)
- Canlı grupta yeni özellik bayraklarını açmak.
- Settings → Alerts: günlük rapor saati 6.
- Groups: demo-retail / demo-hedge hafta sonu kaldıracı.
- İkinci LP sözleşmesi (madde 6'nın gerçek testi).
- White-label alan adı DNS kayıtları (madde 7).

## Durum günlüğü
| Tarih | Madde | Durum | PR | Not |
|---|---|---|---|---|
