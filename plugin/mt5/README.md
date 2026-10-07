# MT5 Server API adaptörü

Bu klasör, `plugin/core`'u MetaQuotes **MT5 Server API**'ye bağlayan DLL'in
kaynağıdır. SDK MetaQuotes tarafından yalnız lisanslı kurumlara verilir; depoda
yoktur. Varsayılan derlemede bu klasör derlenmez (`FXVPS_MT5_SDK=OFF`).

SDK geldiğinde:

```
cmake -S plugin -B build -A x64 -DFXVPS_MT5_SDK=ON -DMT5_SDK_DIR=C:/MT5SDK
cmake --build build --config Release
```

`plugin.cpp` iskeleti, Server API'nin hangi noktasında ne yapılacağını
işaretler. **Arayüz ve metot adları SDK başlıklarıyla birebir teyit
edilmeden derlenmesi beklenmez** — iskelet, adaptörün sorumluluklarını ve
iş parçacığı kurallarını sabitler:

| MT5 tarafı | Adaptör | Çekirdek |
|---|---|---|
| Plugin yüklenir (parametreler: `config` = yapılandırma dosyası yolu) | `Config::parse` | `BridgeCore` kurulur, işçi iş parçacığı `poll()` |
| Yetkili gruptaki işlem isteği (dealer kuyruğu) | `Request` doldurulur | `on_request()` → `Handled` / `NotOurs` / `Local` |
| `NotOurs` / `Local` | istek MT5'e bırakılır | — |
| Dolum | isteği fiyat + hacimle onayla | `IHost::confirm` |
| Ret | isteği uygun dönüş koduyla reddet | `IHost::reject` |
| Fiyat | sembole tick ekle | `IHost::tick` |
| Mutabakat | yönetilen grupların net pozisyonu | `IHost::net_positions` |
| Günlük | sunucu günlüğü | `IHost::log` |

Kurallar:
- MT5 geri çağrılarında **asla ağ beklenmez**; `on_request()` yalnız kuyruğa
  yazar ve hemen döner.
- `IHost` çağrıları yalnız işçi iş parçacığından gelir; MT5 API çağrıları
  oradan yapılır.
- Yönetici, yapılandırma dosyasında `groups=` ile hangi grupların köprüye
  devredildiğini seçer; diğer gruplar hiç etkilenmez.

Örnek yapılandırma: [`fxvps-bridge.conf.example`](fxvps-bridge.conf.example).
