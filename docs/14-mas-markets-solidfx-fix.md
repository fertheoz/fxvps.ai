# MAS Markets (Solid FX) FIX bağlantısı — spec v3.0.3 boşluk analizi

Kaynak: "Solid FX FIX specification 3.0.3" (Nathan Heaney, MAS Markets, 9 Eki 2026). Demo FIX yok; demo yalnız web hesabı (semboller için). Canlı hesap onaylanınca bağlantı testi kalacak.

## Spec özeti
- **FIX 4.2**, iki oturum: Market Data + Order Entry. Oturum saatleri Pazar 17:05 ET – Cuma 17:00 ET; Pzt–Per 17:00–17:05 ET kapalı.
- **Sıra numarası:** MD oturumu her logon'da 1'den başlar; Order Entry oturumu **her işlem günü 17:05 ET'de 1'e döner** (ResetSeqNumFlag=Y önerilmiyor).
- **Logon (A):** 98=0, 108 heartbeat, **554 Password** (553 yok), 141 isteğe bağlı. Mesaj hızı limiti → BusinessMessageReject (j).
- **Semboller:** `55=EUR/USD` metin sembol; LMAX'taki gibi 48 SecurityID yok. Liste `SecurityDefinitionRequest (c)` → `SecurityDefinition (d)` ile çekilir (6666 TickSize, 167 FXSPOT/FXNDF, 64/541 NDF alanları).
- **MD isteği (V):** 262, 263 (1/2), **264** (0 tam kitap / 1 tepe / N kademe), **265=0 zorunlu** (yalnız tam snapshot), 266 Y/N, 110 MinQty, 267/269 (0,1), 146/55.
- **Snapshot (W):** 269/270/271, **290 MDEntryPositionNo, 299 QuoteEntryID zorunlu**, 276 QuoteCondition (B = kapalı), 110 MinQty. Trade entry yok; incremental yok.
- **Emir (D):** 11, 21 (yok sayılır ama zorunlu), 55, 54, **38 OrderQty baz para birimi (CCY1) birim**, 40 (1 Market, 2 Limit, 3 Stop/OCO, Q Trailing), 44, 99, **59** (0 Day, 1 GTC, 3 IOC, 4 FOK, R GFM), 60, 110 MinQty, 210 MaxShow, 117 QuoteID (snapshot'taki 299), 440 ClearingAccount (PB), 15 Currency (terms ccy miktar için), 6668–6673 özel alanlar.
  - Market: yalnız IOC/FOK. Limit: Day/GFM/GTC/IOC/FOK. Stop: Day/GFM/GTC.
- **ExecutionReport (8):** 37, 17, **20 ExecTransType**, 39, 150 (0 New, 4 Cancel, 5 Replace, 6 PendingCancel, 8 Reject, **2 Fill** — hem tam hem kısmi), 32/31 yalnız dolumda, 151/14/6 zorunlu, 103 ret sebebi (1,2,6,11,13,18,99), 64 FutSettDate dolumda.
- **Cancel (F)** 41/11/55/54/60 (+6671 AllowPending), **Replace (G)** 38/44/99 değişebilir, diğer alanlar aynen; **CancelReject (9)** 102 (0,1,6,77,78,99), 434.
- **ListStatusRequest (M) / ListStatus (N):** açık emir listesi (LMAX'taki PositionReport yok!).
- ExecutionAcknowledgment (BN) isteğe bağlı.

## Bizim geçitle (LMAX FIX 4.4) farklar → yapılacaklar
| # | Boşluk | Değişiklik | Yer |
|---|--------|-----------|-----|
| 1 | BeginString sabit `FIX.4.4` | oturum yapılandırmasına `begin_string` (uç nokta başına) | fix-session `SessionConfig`, fix-gateway `Endpoint` |
| 2 | Enstrüman 48/22 (SecurityID) | `InstrumentRef` → `symbol: Option<String>` kipi: encode 55, decode 55 veya 48 | fix-codec `instrument()`/`put_instrument` + normalize (LP sembolü `EUR/USD` zaten bizim iç biçim) |
| 3 | Logon 553+554 | 553 boşken yazılmasın (zaten `opt_str`); `password` ayrı env (`SOLIDFX_PASSWORD`) | config.rs |
| 4 | MD isteği 265 yok | `md_update_type: Some(0)` + `market_depth` (tepe N kademe) uç nokta başına | gateway MD abonelik |
| 5 | Snapshot'ta 290/299/276 | parser bilinmeyen etiketleri atlıyor ✔; 276=B gelirse kitabı boşalt (kapalı piyasa) | normalize.rs |
| 6 | Emir miktarı | birim (units) zaten gönderiyoruz ✔; `Currency` 15 göndermiyoruz ✔ (baz ccy) | — |
| 7 | OrdType/TIF | Market→IOC zorunlu (bizde IOC ✔); GTC limit/stop ✔ (LP'de yatan emirler); FOK gerekirse 59=4 | lp_fix |
| 8 | Sıra sıfırlama 17:05 ET | gelen Logon/mesajda `seq too low` yerine günlük sıfırlamayı kabul: uç nokta başına `daily_reset_et: "17:05"` ya da `reset_on_logon` (önerilmiyor, kayıp riski) | fix-session |
| 9 | Oturum saatleri | 17:00–17:05 ET kopmasını "beklenen kesinti" say; Denetçi/LP-down alarmı bu pencerede susmalı | alerts `lp_down` |
| 10 | **PositionReport yok** | Denetçi LMAX'ta AP ile mutabakat yapıyor; Solid için `ListStatus (M/N)` + kendi dolum bandımız; held-net yalnız ExecutionReport'tan | denetci.rs: LP başına "mutabakat kaynağı" |
| 11 | Sembol listesi | `SecurityDefinitionRequest` ile çek ve `solidfx-instruments.txt` üret; tick size 6666'dan `digits` | yeni araç `deploy/lmax-demo/solidfx-semboller.sh` |
| 12 | ClearingAccount 440 | onboarding'de verilirse uç nokta ayarı `clearing_account` | config + NewOrderSingle |
| 13 | Hız limiti → `j` | BusinessMessageReject'i LP reddi gibi işle, emri yeniden deneme (retry politikası) | gateway |
| 14 | ExecType=2 hem kısmi hem tam | parser OrdStatus 1/2 ile ayırıyor ✔ | — |

**Durum (9 Eki, PR "solidfx codec 1"):** 1 (`begin_string` geçit ayarı), 2 (`security_id_source = "SYMBOL"` kipi: 55 yazılır/okunur, 48/22 yok, 21 eklenir), 3 (553 zaten isteğe bağlı) hazır; şablon `deploy/lmax-demo/lp-solidfx-gateway.example.json`. **2. dilim:** 8 (oturum katmanı karşı tarafın logon'daki sıra yeniden başlatmasını zaten alıyor — regresyon testi eklendi), 9 (`session_hours`: saat dışında bağlanma/alarm yok, `scheduled` düşüş), 10'un geçit yarısı (`OrderCommand::ListStatus` → M, N yanıtı `LpMessage`). Kalan: 4 (derinlik), 5, 10'un Denetçi yarısı, 11–13.

Tahmin: 1–5, 7, 11 bir PR (fix-codec/fix-session/gateway, ~1 gün); 8–10 ikinci PR (oturum takvimi + Denetçi kaynağı); 12–13 küçük.

## Bugün yapılabilecekler (FIX demo olmadan)
1. Codec/oturum değişikliklerini (1–5) yazıp **lp-simulator'a FIX 4.2 / Symbol kipi** ekleyerek sınamak (simülatör zaten bizde).
2. Demo web hesabından sembol/sözleşme listesini alıp `solidfx-instruments.txt` taslağı (spec'te liste yok; Nathan'dan "symbol/contract specification list" bekleniyor).
3. Nathan'a sorulacaklar: SenderCompID/TargetCompID ve host:port (TLS?), MD derinliği (264) ve MinQty, ClearingAccount gerekli mi, ListStatus ile mutabakat, hız limiti değeri, Account (1) beklentisi.

Not: paylaşılan demo kullanıcı adı/parolası bu depoya **yazılmaz**.
