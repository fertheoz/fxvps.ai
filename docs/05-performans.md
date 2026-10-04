# 05 — Performans: client-gateway yük testi

Araç: `tools/loadgen` (Rust). N eşzamanlı WebSocket istemcisi açar; her biri `Hello` + `Auth` (JWT) gönderir, sembollere abone olur ve isteğe bağlı olarak sabit hızla küçük piyasa emirleri (alış/satış dönüşümlü, net pozisyon ≈ 0) verir.

Ölçülenler:
- **Quote fan-out gecikmesi** = istemcinin alış anı − quote'un `ts_ns` alanı (duvar saati; yalnız istemci ve sunucu aynı makinede/saatte iken anlamlı). Sunucu tarafı conflation penceresini (`max_quote_hz`) **içerir**.
- **Emir ack / fill gecikmesi** = `PlaceOrder` gönderimi → `Ack` / ilk `FILLED` `OrderUpdate`.
- Hata sayaçları: bağlanma, auth, sunucu `ErrorCode`'ları (ör. `server_rate_limited`), red, 5 sn'de son duruma ulaşmayan emir.

```bash
# Süreç içi demo (lp-simulator + fix-gateway + core-engine + client-gateway, DEV anahtarı):
cargo run --release -p loadgen -- --spawn-demo --clients 50 --duration 30 --orders-per-sec 0.5
# Çalışan bir gateway'e karşı (jeton: --token, $FXVPS_TOKEN ya da $FXVPS_JWT_HS256_SECRET ile hesap başına basılır):
cargo run --release -p loadgen -- --url ws://127.0.0.1:8080/ws --clients 200 --duration 60 --json
```

Parametreler: `--symbols EURUSD,GBPUSD,USDJPY`, `--quote-hz`, `--qty` (baz birim, varsayılan 1000), `--accounts DEMO-1,DEMO-2,DEMO-3` (istemciler hesaplara döngüyle dağıtılır), `--ramp-ms`. Emir hızı hesap başına sınırın (`orders_per_second`, varsayılan 10/sn) altında tutulmalı; aksi hâlde `server_rate_limited` sayılır.

## Sonuç: bulut sandbox'ı, 4 Eki 2026

> **SANDBOX SAYILARI** — paylaşılan, aşırı yüklü bir bulut konteyneri (4 vCPU, başka derlemeler eşzamanlı), istemci ve sunucu aynı süreçte. Kapasite planlaması için **kullanılmaz**; yalnız aracın çalıştığını ve büyüklük mertebesini gösterir. Gerçek ölçüm ayrı istemci/sunucu makineleriyle yapılmalıdır.

Kurulum: `--spawn-demo` (gateway `max_quote_hz = 20`, simülatör tick 250 ms, 3 sembol), 50 istemci, 30 sn, istemci başına 0,5 emir/sn (toplam ≈ 25 emir/sn, 3 hesaba dağıtılmış), release derleme.

| Ölçüm (ms) | adet | p50 | p95 | p99 | maks | ort. |
|---|---:|---:|---:|---:|---:|---:|
| Quote fan-out | 17 829 | 24,4 | 47,6 | 49,9 | 51,8 | 24,7 |
| Emir ack | 700 | 0,86 | 1,23 | 2,02 | 41,1 | 0,92 |
| Emir fill | 700 | 1,68 | 44,5 | 47,7 | 48,7 | 10,1 |

50/50 istemci bağlandı, 594 quote/sn alındı, 701 emir (700 ack, 700 fill, 0 red; 1 emir süre bitiminde yoldaydı), hata yok. İkinci koşum (tablo çıktısı) aynı mertebede: quote p50 20,8 / p99 49,2 ms; ack p50 0,86 / p99 2,4 ms; fill p50 1,66 / p99 48,0 ms.

Yorum:
- Quote gecikmesi ≈ 0–50 ms düzgün dağılım: 20 Hz conflation penceresi (50 ms) baskın; ağ/işleme payı ölçülemeyecek kadar küçük. Daha düşük gecikme için `max_quote_hz` artırılmalı (bant genişliği karşılığında).
- Fill p50 ≈ 1,7 ms ama p95 ≈ 45 ms: emirlerin bir kısmı simülatörün/gateway'in toplu işleme aralığına denk geliyor olabilir; çift tepeli dağılımın kaynağı (simülatör tick'i mi, OMS/FIX yolu mu) ayrı incelenmeli.
- Ack (risk + OMS kabulü) 1 ms altında.

Sonraki adımlar: ayrı makinelerden 1k/5k/10k bağlantı, `max_quote_hz` taraması, gateway CPU/bellek ve `/metrics` (yavaş tüketici, düşen çerçeve) ile birlikte raporlama; CI'de küçük bir duman koşumu (`cargo test -p loadgen` zaten 4 istemcilik uçtan uca test içerir).
