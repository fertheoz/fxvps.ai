# 07 — Güvenlik düzeltmeleri (G1–G17)

Kaynak: `docs/06-guvenlik-incelemesi.md` (dal `feat/hardening`, PR #14; bu dalda yok).
Bu dal: `fix/security-findings` (taban `main` @ `1ef6920`). Her bulgu için: ne değişti,
hangi commit, hangi test, ne kaldı. Durum: **Düzeltildi** / **Kısmi** / **Ertelendi**.

## Özet

| # | Önem | Durum | Commit |
|---|---|---|---|
| G1 | Kritik | Düzeltildi | client-gateway: `7509689`; Helm: `738f5f3` |
| G2 | Kritik | Düzeltildi (derleme özelliğine taşınmadı, bkz. kalanlar) | `3dfdee5` |
| G3 | Yüksek | Düzeltildi | `0b351f9` |
| G4 | Yüksek | Düzeltildi | `7f3142b` |
| G5 | Yüksek | Düzeltildi | `3dfdee5` (+ Helm `738f5f3`) |
| G6 | Yüksek | Düzeltildi (HttpOnly yenileme çerezi yok, bkz. kalanlar) | core-engine `3dfdee5`; back office + nginx `3e41eab` |
| G7 | Orta | Düzeltildi | `7f3142b` |
| G8 | Orta | Kısmi | `7f3142b` |
| G9 | Orta | Düzeltildi | `7509689` |
| G10 | Orta | Kısmi (identity chart'ta yok) | `738f5f3` (metrik portu: `7509689`) |
| G11 | Orta | Düzeltildi (Deployment + PVC; StatefulSet değil) | `738f5f3` |
| G12 | Orta | Düzeltildi | `4a1383e` |
| G13 | Düşük | Kısmi (terminal meta CSP yok) | `3e41eab` |
| G14 | Düşük | Ertelendi | — |
| G15 | Düşük | Ertelendi | — |
| G16 | Düşük | Düzeltildi | `7f3142b` |
| G17 | Bilgi | Tamam (çalışma zamanı imajları digest; SBOM + provenance + cosign keyless) | `fix/g17` |

## Ayrıntılar

### G1 — client-gateway DEV anahtarı
- `services/client-gateway/src/auth.rs`: `dev_key_allowed()` + `Authenticator::enforce_dev_key_policy(demo)`.
  `main.rs` başlangıçta çağırır: anahtar kaynağı yoksa (`FXVPS_JWT_JWKS_URL` / `_JWKS_FILE` /
  `_RS256_PUBLIC_KEY_FILE` / `_HS256_SECRET`) yalnız `--demo` veya açık `FXVPS_DEV_AUTH=1` ile
  başlar; aksi hâlde hata ile çıkar.
- Helm: `auth.jwksUrl` / `auth.issuer` / `auth.audience` → `FXVPS_JWT_JWKS_URL`,
  `FXVPS_JWT_ISSUER`, `FXVPS_JWT_AUDIENCE` (`required`: değer yoksa şablon üretilmez).
- Test: `auth::tests::dev_key_needs_demo_or_explicit_flag`.

### G2 — core-engine `/auth/dev-token`
- `admin/auth.rs`: `dev_auth_policy()` — `CORE_DEV_AUTH=1` herhangi bir `CORE_JWT_*` anahtarıyla
  birlikte verilirse veya `CORE_ADMIN_ADDR` loopback değilse başlatma reddedilir.
- Dev imzalayıcı **süreç başına rastgele 32 baytlık anahtar** kullanır (`with_random_dev_signer`);
  yapılandırılmış sır asla dev jetonu imzalamaz. Dev jetonları `iss = core-engine-dev` taşır ve
  yalnız dev anahtarıyla, üretim anahtarları yalnız üretim `iss`/`aud` ile doğrulanır. Sabit
  `DEV_HS256_SECRET` kaldırıldı.
- `admin` rolü yalnız `CORE_DEV_AUTH_ADMIN=1` ile basılır (aksi hâlde 403). Süre 12 sa → 8 sa.
- **Dört göz:** dev-token isteğindeki `sub` yok sayılır; sunucu `dev-<rol>` türetir. Üretimde
  `sub`, imzası ve `iss`/`aud`'u doğrulanmış identity kullanıcı kimliğidir (çağıran seçemez);
  onay kuralı (`routes.rs` `approve`) bu sunucu tarafı kimliği karşılaştırır. Dev modunda dört göz
  yalnız rol başınadır (belgelendi: `admin/auth.rs` modül belgesi, `API.md`).
- Testler: `admin::auth::tests::{dev_auth_policy_rules, dev_signer_is_separate_and_admin_gated}`,
  `tests/admin_api.rs::dev_token_flow` (admin 403, `sub` sunucu türetir).

### G3 — terminal jeton sızdırma bağlantısı
- `apps/terminal/src/store/connection.ts`: `isAllowedWsUrl()` — izinli kökenler: derleme zamanı
  `VITE_ALLOWED_WS_ORIGINS` (virgüllü), sayfanın kendi kökeni (`https:` → `wss:` aynı host) ve
  yalnız dev derlemesinde ya da sayfa loopback'ten sunuluyorsa loopback gateway'ler.
  `?api=ws&url=` izinli değilse yok sayılır (`console.warn`), saklanmaz.
- `store/api.ts`: identity oturum jetonu yalnız izinli kökene gönderilir; elle girilen yabancı
  gateway'e yalnız kullanıcının yapıştırdığı jeton gider.
- `infra/docker/static-web.Dockerfile`: `VITE_ALLOWED_WS_ORIGINS`, `VITE_IDENTITY_URL`,
  `NEXT_PUBLIC_API_URL` derleme argümanları.
- Test: `src/store/connection.test.ts` (izin listesi, alt alan adı taklidi, link reddi).

### G4 — identity posta
- `services/identity/src/mail.rs`: `SmtpMailer<T: lettre::AsyncTransport>` (`IDENTITY_SMTP_URL`,
  `IDENTITY_MAIL_FROM`), `select_mailer()`: SMTP varsa SMTP; `LogMailer` yalnız `DATABASE_URL`
  yokken (geliştirme) veya açık `IDENTITY_DEV_MAILER=1` ile; aksi hâlde başlatma reddedilir.
- `LogMailer` artık log'a bağlantı/gövde yazmaz (yalnız alıcı + konu); bağlantılar yalnız
  `IDENTITY_DEV_MAIL_FILE` dosyasına (e2e).
- Testler (sahte taşıma `AsyncStubTransport`): `mail::tests::{log_mailer_only_in_dev,
  smtp_mailer_sends_through_transport, smtp_errors_propagate_and_bad_input_is_rejected}`.

### G5 — core-engine admin JWT
- Birincil üretim yolu identity JWKS: `CORE_JWT_JWKS_URL` (RS256/ES256, `kid`; arka planda her
  `CORE_JWT_JWKS_REFRESH_SECS`=300 sn yenilenir, hata hâlinde 5 sn'de bir yeniden dener) veya
  `CORE_JWT_JWKS_FILE`. `CORE_JWT_ISSUER` / `CORE_JWT_AUDIENCE` verilince `iss`/`aud` zorunlu ve
  denetlenir (verilmezse uyarı). Leeway 30 → 5 sn.
- identity `roles` dizisi desteklenir (en yetkili arka ofis rolü seçilir; yalnız `client` → 401).
- `CORE_REQUIRE_MFA=1`: `.view` olmayan her izin ve legacy uçlar `amr` içinde `otp|mfa|hwk|swk`
  ister (403 `mfa_required`). Helm `auth.requireMfa: true`.
- Testler: `admin::auth::tests::{iss_aud_leeway_enforced, identity_roles_and_mfa}`,
  `tests/admin_jwks.rs` (identity `KeyRing` ile RS256, yanlış iss/aud/anahtar, MFA, JWKS URL).

### G6 — back office jeton saklama + SSE + CSP
- `apps/backoffice/src/lib/auth.ts`: jeton bellek + `sessionStorage` (yalnız bu sekme);
  `localStorage` kullanılmaz, eski sürümden kalan anahtar ilk okumada silinir.
- SSE: `POST /v1/stream/ticket` (bearer başlıkla) → 30 sn, tek kullanımlık rastgele bilet;
  `GET /v1/stream?ticket=`. `?access_token=` kaldırıldı (`admin/stream.rs`). Başlıklı istek
  (fetch tabanlı SSE) hâlâ kabul edilir.
- CSP: `apps/backoffice/scripts/csp.mjs` derleme sonrası her HTML'e `<meta>` CSP ekler: satır içi
  betikler yalnız SHA-256 özetleriyle, `connect-src 'self' <NEXT_PUBLIC_API_URL kökeni>`,
  `object-src 'none'`, `base-uri 'self'`. `pnpm build` ve `build-live.mjs` çalıştırır.
- nginx: `infra/docker/nginx-security-headers.conf` (CSP `frame-ancestors 'none'`,
  `X-Frame-Options`, HSTS, `Permissions-Policy`, COOP, `nosniff`); her `add_header` bloğunda
  yeniden dahil edilir (G13'teki kalıtım sorunu).
- Testler: `tests/http-api.test.ts` (bilet akışı, URL'de jeton yok), `tests/token-storage.test.ts`,
  `tests/csp.test.ts`, core-engine `admin_api.rs::live_stream_and_cors` (URL jetonu 401, bilet tek
  kullanımlık).

### G7 — `X-Forwarded-For`
- `services/identity/src/http.rs`: `forwarded_client_ip(xff, hops)` — **sağdan** `IDENTITY_TRUSTED_PROXY_HOPS`
  (varsayılan 1) giriş; geçersiz/eksikse soket adresi. Test: `xff_tests::rightmost_trusted_hop_wins`.

### G8 — hız sınırı / TOTP
- `/v1/sessions/revoke-all`, `/v1/2fa/totp/*`, `/v1/passkeys/register/*`, `/v1/admin/*` artık IP
  hız sınırlayıcısının arkasında.
- `totp_confirm` / `totp_disable`: kilit denetimi + hatalı kod `register_failure` (kilitleme eşiği).
- `/v1/admin/*` kullanıcı jetonuyla MFA (`amr` ∋ `otp|mfa|hwk`) ister (`IDENTITY_ADMIN_REQUIRE_MFA`,
  varsayılan açık); servis jetonu muaf.
- Testler: `flows.rs::{totp_disable_guessing_locks_the_account,
  admin_routes_need_mfa_and_sensitive_routes_are_rate_limited}`.
- **Kalan:** kullanıcı başına (jeton `sub`) ayrı sınırlayıcı yok (IP başına); 2FA kaldırmak için
  parola + kod şartı eklenmedi (API değişikliği; terminal UI'ı etkiler).

### G9 — client-gateway oturum ve kaynak sınırları
- `conn.rs`: `exp` anında bağlantı `Error(UNAUTHENTICATED, "token expired")` + kapanış 4001 ile
  kapanır (istemci yeni jetonla yeniden bağlanır).
- `limits.rs` `ConnLimits`: küresel (`max_connections` 10000), IP başına (`max_connections_per_ip`
  50, yükseltmeden önce HTTP 429) ve `sub` başına (`max_connections_per_subject` 20, `Auth` sonrası
  `RATE_LIMITED` + kapanış 4009). `serve()` `ConnectInfo` ile çalışır.
- `Origin` izin listesi (`allowed_origins`; boş = herkes, `Origin`'siz yerel istemci kabul).
- Ortam: `FXVPS_METRICS_LISTEN`, `FXVPS_ALLOWED_ORIGINS`, `FXVPS_MAX_CONNECTIONS[_PER_IP|_PER_SUBJECT]`.
- Testler: `tests/session_limits.rs`, `limits::tests::caps_and_release`,
  `conn::tests::token_deadline_is_relative_to_exp`, `config::tests::env_overrides`.
- Not: ters vekil arkasında IP başına sınır vekil adresini görür; vekilde de bağlantı sınırı önerilir.

### G10 — Helm ağ politikası / metrikler / sırlar
- `templates/networkpolicy.yaml`: `policyTypes: [Ingress, Egress]`; egress yalnız DNS, NATS,
  `egressTo` (chart içi servisler), `egressIdentity` (JWKS) ve `egressCIDRs` (ör. gerçek LP).
- client-gateway `/metrics` ayrı portta (`metrics: 9090`, `FXVPS_METRICS_LISTEN`); herkese açık
  ingress yalnız `http` portuna, metrik portuna yalnız Prometheus.
- `secretRef optional: false`.
- **Kalan:** identity servisi chart'a eklenmedi (ayrı iş; şu an `networkPolicy.identity` seçicisiyle
  harici/başka chart varsayılıyor).

### G11 — securityContext / kalıcılık
- Pod: `runAsUser/runAsGroup/fsGroup: 65532` (distroless `nonroot`), statik web `101`.
- core-engine: `persistence` (PVC `core-engine-data`, 20Gi, `helm.sh/resource-policy: keep`)
  `/home/nonroot/data`'ya bağlı; `Recreate`. `CORE_ADMIN_ADDR=0.0.0.0:8080` (eskiden loopback'te
  dinlediği için servis erişilemezdi).
- **Kalan:** StatefulSet yerine Deployment + RWO PVC (replicas 1, single-writer).

### G12 — CI tedarik zinciri
- Tüm eylemler tam commit SHA'sına sabitlendi (`# vX` yorumuyla; SHA'lar
  `git ls-remote https://github.com/<eylem>.git refs/tags/<etiket>` ile, açıklamalı etiketlerde
  `^{}` commit'i). `dtolnay/rust-toolchain` SHA ile `toolchain: stable` girdisi alır.
- `ci.yml`: üst düzey `permissions: contents: read`.
- `docker.yml`: iş akışı düzeyi `contents: read`; PR'da `build` işi (kayıt girişi yok, push yok,
  `packages` yok), `packages: write` yalnız push/manuel `publish` işinde.
- `.github/dependabot.yml` (github-actions, haftalık).

### G13 — statik web başlıkları
- G6'daki nginx başlık dosyası (CSP `frame-ancestors`, HSTS, `Permissions-Policy`, her `location`).
- **Kalan:** terminal için sayfa CSP'si (`script-src`/`connect-src`) eklenmedi; e2e'nin yerel
  gateway/identity adresleriyle birlikte tasarlanmalı.

### Ertelenenler
- **G14** (erişim jetonu 5 dk durumsuz, bootstrap admin kayıtta): değişiklik yok. Öneri: admin
  rolleri için ≤ 60 sn TTL, bootstrap'ı tek seferlik CLI'ya taşımak.
- **G15** (`CORE_CORS_ORIGINS=*`): değişiklik yok. Öneri: `CORE_DEV_AUTH` dışında `*`'ı reddet.
- **G17** tamamlandı: distroless/nginx çalışma zamanı imajları digest ile sabit (Dependabot docker); yayında SBOM, `provenance: mode=max` ve cosign keyless imza. Doğrulama: `cosign verify ghcr.io/fertheoz/fxvps-<svc>@<digest> --certificate-identity-regexp "^https://github.com/fertheoz/fxvps.ai/" --certificate-oidc-issuer https://token.actions.githubusercontent.com`. Derleme imajları (cargo-chef, node) sabitlenmedi.
- **G2 derleme özelliği:** dev-auth `--features dev-auth` arkasına alınmadı; bunun yerine çalışma
  zamanı kapıları (loopback, üretim anahtarıyla birlikte ret, rastgele anahtar, admin bayrağı).

## Doğrulama
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` (+ CI özellik
  varyantları), `cargo test --workspace`.
- terminal ve back office: `typecheck`, `lint`, `test`, `build`.
- `helm lint` + `helm template` (client-gateway/core-engine etkin, `auth.*` verilmeden hata).
- `actionlint`.
- Uygulanmadı: Docker imaj derlemesi / nginx `-t` (bu ortamda docker daemon yok), Playwright e2e.
