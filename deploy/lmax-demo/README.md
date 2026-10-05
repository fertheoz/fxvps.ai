# LMAX demo bağlantı paketi

fix-gateway'i LMAX **demo** ortamına FIX 4.4 ile bağlamak için geçici kurulum.
Kurulum mt5forexvps altyapısında, ödünç bir LXC'de yapılır; adımlar
[`BRIFING-mt5forexvps.md`](BRIFING-mt5forexvps.md) dosyasında.

| Dosya | İçerik |
|---|---|
| `lmax-demo.toml` | Gateway ayarı; `<...>` alanları LMAX demo hesabının FIX bilgileriyle doldurulur |
| `.env.example` | Parola değişkenleri (değer yok); hedefte `.env` olarak kopyalanır, `chmod 600` |
| `docker-compose.yml` | Yalnız fix-gateway; durum ucu `127.0.0.1:9890/status` |
| `BRIFING-mt5forexvps.md` | mt5forexvps masaüstü oturumu için iş tanımı |

Parolalar depoya, sohbete veya bilet sistemine yazılmaz.
