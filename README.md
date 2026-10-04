# fxvps.ai

Modern, çok kiracılı (multi-tenant) trading platformu: likidite sağlayıcılarına (ör. LMAX) FIX API ile bağlanır, son kullanıcılara alt hesaplar ve MetaTrader 5 / cTrader sınıfında süper modern bir trading terminali sunar.

- `docs/` — araştırma, mimari ve yol haritası (M0)
- `crates/` — Rust kütüphaneleri: `domain` (fixed-point tipler), `fix-codec` (FIX 4.4), `fix-session` (oturum katmanı)
- `services/` — `lp-simulator` (LMAX benzeri FIX acceptor), `fix-gateway` (LP'ye initiator, MD normalize + emir)
- `apps/`, `packages/` — gelecekteki TypeScript uygulamaları (pnpm + Turborepo iskeleti)

## M1: FIX gateway — çalıştırma

Gereksinim: Rust stable (≥ 1.87; `rust-toolchain.toml`). NATS testler için gerekmez.

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                     # unit + proptest + session + uçtan uca (simülatör + gateway)

# İki terminalde:
cargo run -p lp-simulator      # services/lp-simulator/config/default.toml, MD :9880, trading :9881
RUST_LOG=info,fix_gateway=debug cargo run -p fix-gateway   # services/fix-gateway/config/default.toml

# NATS yayıncısıyla (config'te [nats] bölümünü aç):
cargo run -p fix-gateway --features nats
# TLS taşıma katmanı:
cargo build -p fix-session --features tls
```

Gerçek LP parolaları depoya girmez: `FIX_GATEWAY_MD_PASSWORD` / `FIX_GATEWAY_TRADE_PASSWORD`
ortam değişkenleri config'teki değeri ezer. Örnek config'lerdeki `demo/demo` yalnız yerel simülatör içindir.

LMAX'e özgü ayrıntılar (CompID'ler, `SecurityIDSource=8`, EUR/USD dışındaki enstrüman ID'leri,
Logon'da Username/Password, taker için yalnız IOC/FOK) **varsayımdır**; docs/02'deki `[DOĞRULA]`
maddeleri LMAX spesifikasyonu ile kapatılmalıdır.

Geliştirme yalnız PR üzerinden yürür; `main` doğrudan push almaz.
