# services/

Çalıştırılabilir Rust servisleri (binary crate'ler). Kütüphaneler `crates/` altında.

- `lp-simulator/` — LMAX benzeri FIX 4.4 acceptor simülatörü (MD + trading oturumları)
- `fix-gateway/` — LP'ye bağlanan FIX initiator; MD'yi normalize eder, iç emir komutlarını kabul eder
