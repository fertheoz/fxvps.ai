# TradingView uyarı köprüsü (parça 16, kademe 1)

Müşteri Pine stratejisini TradingView'da yazar; uyarı (alert) webhook'u bizim REST API'ye gelir, emir bizde açılır. Platform `api` sayılır: yönlendirme kuralları, last look, hız limiti ve API anahtarı kapsamı aynen uygulanır.

## Uç nokta
`POST https://<api>/api/v1/webhooks/tradingview` — `Authorization` başlığı **yok** (TradingView başlık gönderemez); kişisel API anahtarı (trade kapsamı) gövdedeki `token` alanında taşınır.

## Uyarı mesajı (TradingView "Message" alanı)
Açılış:
```json
{"token":"<API anahtarı>","symbol":"{{ticker}}","side":"{{strategy.order.action}}","qty":"100000","client_order_id":"tv-{{timenow}}","sl":"{{plot(\"sl\")}}","tp":"{{plot(\"tp\")}}"}
```
Kapanış (sembolün tüm pozisyonları):
```json
{"token":"<API anahtarı>","action":"close","symbol":"{{ticker}}","client_order_id":"tvc-{{timenow}}"}
```
Alanlar: `action` (`open` varsayılan | `close`), `account` (anahtar birden çok hesap taşıyorsa), `symbol`, `side` (`buy`/`sell`), `type` (`market` varsayılan | `limit` | `stop`), `qty` (baz birim), `limit_price`, `stop_price`, `sl`, `tp`, `tif`, `position_id` (yalnız o pozisyonu kapat), `client_order_id` (idempotency: aynı uyarı iki kez gelirse ikinci `replayed: true` ile 200 döner).

Yanıtlar: 201 (açıldı), 200 (tekrar / kapatıldı; `closed[]`), 400 (eksik alan / bilinmeyen `action`), 401 (geçersiz token), 403 (hesap anahtara ait değil), 429 (hız limiti).

## Notlar
- Sembol bizim sembol adımızla eşleşmeli (`EURUSD`, `XAUUSD`); TradingView'da `{{ticker}}` "XAUUSD" verir, "OANDA:XAUUSD" gibi önekler için uyarı metnine sabit sembol yazın.
- `qty` baz para birimi birimidir (1 lot EURUSD = 100000; 1 lot XAUUSD = 100).
- TradingView'ın webhook IP'leri sabittir; istenirse yük dengeleyicide allowlist uygulanabilir (uygulanmadı; token yeterli).
- Robot altyapısı (kademe 2: terminal içi strateji betiği + backtest) ve sunucuda barındırma (kademe 3) ayrı parçalar.
