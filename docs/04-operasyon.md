# 04 — Operasyon (fxvps.ai)

> Durum: taslak (M1). Altyapı dosyaları: `infra/docker/`, `deploy/compose/`, `deploy/helm/fxvps/`, `.github/workflows/{docker,pages}.yml`.

## 1. Yerel geliştirme

Gereksinim: Docker (Compose v2), Rust stable, Node 22 + pnpm 9.

```bash
cp deploy/compose/.env.example deploy/compose/.env        # yalnız yer tutucu değerler
docker compose -f deploy/compose/docker-compose.yml up -d --build
docker compose -f deploy/compose/docker-compose.yml ps
docker compose -f deploy/compose/docker-compose.yml logs -f fix-gateway
docker compose -f deploy/compose/docker-compose.yml down        # -v: verileri de siler
```

| Servis | Adres (yalnız 127.0.0.1) | Not |
|---|---|---|
| PostgreSQL 17 | :5432 | ledger/hesaplar |
| Valkey | :6379 | cache/pubsub |
| NATS JetStream | :4222, izleme :8222 | olay omurgası |
| QuestDB | web :9000, ILP :9009, PG :8812 | ham tick |
| ClickHouse | HTTP :8123, native :9440 | analitik |
| OTel collector | OTLP :4317/:4318 | metrik → Prometheus |
| Prometheus | :9090 | |
| Grafana | :3000 | "fxvps — Genel Bakış" panosu hazır gelir |
| lp-simulator | MD :9880, trading :9881 | |
| fix-gateway | — | simülatöre bağlanır, NATS'e yayınlar (`nats` özelliğiyle derlenir) |

Compose'a özgü config'ler `deploy/compose/config/*.toml` (servis adları, 0.0.0.0). Rust servislerini konteyner dışında koşmak için README'deki `cargo run` komutları geçerlidir; altyapı servisleri compose'tan kullanılabilir.

Web uygulamaları her biri kendi lock dosyasıyla kurulur:
```bash
cd apps/terminal   && pnpm install --ignore-workspace --frozen-lockfile && pnpm dev
cd apps/backoffice && pnpm install --ignore-workspace --frozen-lockfile && pnpm dev
```

## 2. Ortamlar

| Ortam | Yer | Dağıtım | Veri |
|---|---|---|---|
| dev | geliştirici makinesi | docker compose | sentetik (lp-simulator) |
| preview | GitHub Pages | `pages.yml` (main push) — terminal `/fxvps.ai/`, back office `/fxvps.ai/admin/` | yalnız statik demo |
| staging | Kubernetes (bulut) | Helm `deploy/helm/fxvps`, imajlar `ghcr.io/fertheoz/fxvps-<svc>:sha-<kısa>` | LP UAT/demo hesabı |
| prod | LD4 bare-metal (FIX GW + core) + Kubernetes (kenar, API, UI) | etiketli sürüm, elle onay | gerçek |

İmajlar: PR'da yalnız derlenir; `main` push'unda GHCR'a `sha-*` ve `latest` etiketiyle itilir. Henüz main'de olmayan servisler (client-gateway, core-engine) iş akışında atlanır. Prod'a `latest` değil, sabit `sha-*` etiketi (tercihen digest) dağıtılır.

## 3. Colocation notu (LD4)

- LMAX'in Londra eşleşme motoru Equinix **LD4**'tedir; FIX gateway ve core (OMS/risk) aynı veri merkezinde bare-metal sunucuda koşar. Kubernetes'te CPU pinning ve ağ jitter'ı p99'u bozar (docs/03 §1).
- Sunucu ayarı: `isolcpus`/`nohz_full` ile ayrılmış çekirdekler, IRQ affinity, C-state kapalı, performans governor, NUMA yerelliği; PTP/chrony ile saat senkronu (MiFID II RTS 25).
- LP bağlantısı: cross-connect (tercih) veya LP'nin sağladığı VPN/extranet; FIX üzerinde TLS (`fix-session --features tls`).
- LD4 ↔ bulut: NATS leaf node üzerinden, mTLS; LD4 dışa yalnız giden bağlantı açar.
- Dağıtım: aynı OCI imajı systemd + podman (`--network host`, `--cpuset-cpus`) ile, ya da imajdan çıkarılan statik ikili; Helm yalnız bulut tarafı içindir. `fix-gateway` tekildir (aynı CompID ile iki oturum açılamaz) — aktif/pasif, otomatik değil kontrollü devir.
- FIX sequence store (`store_dir`) kalıcı yerel diskte; devirde store pasif düğüme taşınır.

## 4. Sır yönetimi

- Depoya (public) hiçbir sır girmez. `.env.example` ve örnek config'lerdeki değerler yalnız yerel yer tutucudur (`demo/demo`, `dev-only-change-me`).
- **Kısa vade: SOPS + age.** Ortam başına şifreli `secrets/<ortam>.enc.yaml` ayrı özel depoda; age anahtarları kişi başına, CI anahtarı yalnız dağıtım işinde. Kubernetes'e `helm-secrets` veya External Secrets ile `Secret` olarak iner; chart yalnız `existingSecret` adını bilir (ör. `fix-gateway-lp-credentials` → `FIX_GATEWAY_MD_PASSWORD`, `FIX_GATEWAY_TRADE_PASSWORD`).
- **Orta vade: HashiCorp Vault (veya OpenBao).** Kubernetes auth + External Secrets Operator; DB için dinamik kimlik bilgileri; PKI motoru ile servisler arası mTLS sertifikaları; audit log açık.
- FIX/LP parolaları ve imza anahtarları: KMS/HSM korumalı, erişim 4-eyes; LD4 sunucusunda yalnız tmpfs'e açılır, ortam değişkeni olarak sürece verilir.
- CI: GHCR için yalnız `GITHUB_TOKEN` (`packages: write`); uzun ömürlü token yok. Sızıntı şüphesinde: döndür → iptal et → audit log incele.

## 5. Gözlemlenebilirlik

- Servisler OTLP ile OTel collector'a yollar (metrik/iz/log); collector metriği Prometheus'a açar. Üretimde iz → Tempo, log → Loki, metrik → Mimir/Prometheus (docs/03).
- Kubernetes'te prometheus-operator varsa `serviceMonitor.enabled=true`.
- Temel SLI'lar: FIX oturum durumu (logon/heartbeat/seq gap), tick gecikmesi (LP SendingTime → yayın, HdrHistogram p50/p99/p99.9), emir gidiş-dönüş, reject oranı, NATS consumer lag, DB replikasyon gecikmesi.
- Alarmlar: FIX oturumu düştü > 10 sn, tick akışı yok > 5 sn (piyasa açıkken), p99 eşiği aşıldı, ledger mutabakat farkı ≠ 0, disk > %80.

## 6. Yedekleme / geri yükleme

| Veri | Yöntem | RPO / RTO hedefi |
|---|---|---|
| PostgreSQL (ledger) | sürekli WAL arşivi + günlük tam yedek (pgBackRest/WAL-G → S3 uyumlu, şifreli, farklı bölge) | RPO ≤ 1 dk / RTO ≤ 1 sa |
| NATS JetStream | stream replikası R3 + periyodik `nats stream backup` | RPO ≤ 5 dk |
| ClickHouse | `BACKUP ... TO S3` günlük | RPO 24 sa (yeniden türetilebilir) |
| QuestDB | anlık görüntü (`SNAPSHOT PREPARE` + disk snapshot) | RPO 24 sa |
| FIX seq store | dosya kopyası her devirde + günlük | — |

Yerel örnek:
```bash
docker compose -f deploy/compose/docker-compose.yml exec postgres pg_dump -U fxvps -Fc fxvps > fxvps.dump
docker compose -f deploy/compose/docker-compose.yml exec -T postgres pg_restore -U fxvps -d fxvps --clean < fxvps.dump
```
Geri yükleme ayda bir staging'de prova edilir; prova edilmemiş yedek yedek sayılmaz.

## 7. Olay (incident) runbook'u

1. **Algıla ve ilan et**: alarm → nöbetçi kanalda olay açar, şiddet atar (SEV1: emir alınamıyor/para riski; SEV2: bozulma; SEV3: tekil).
2. **Koru**: SEV1'de kill-switch — yeni emirleri durdur (client-gateway'de salt-okuma kipi), açık pozisyonlara dokunma; gerekirse LP tarafında oturumu kontrollü kapat.
3. **Teşhis**: Grafana panosu, `fix-gateway` logları (seq gap, reject, logout nedeni), NATS consumer lag, son dağıtım (`sha-*` etiketi).
4. **Düzelt**: son dağıtımsa önceki imaj etiketine dön (`helm rollback fxvps <rev>` / LD4'te önceki imaj); FIX seq sorununda ResendRequest/SequenceReset LP ile koordineli.
5. **Mutabakat**: olay penceresindeki emir/ExecID'leri LP drop-copy/statement ile karşılaştır; ledger farkı sıfırlanmadan olay kapanmaz.
6. **Kapanış**: 5 iş günü içinde suçlamasız postmortem (zaman çizelgesi, kök neden, eylem maddeleri); müşteri etkisi varsa iletişim ve gerekirse düzenleyici bildirimi.

Örnek senaryolar:
- *FIX oturumu düşüyor*: ağ/cross-connect → LP durum sayfası → logon reddi (parola/CompID) → seq uyumsuzluğu (store'u silme; LP ile reset kararlaştır).
- *Tick akışı durdu, oturum ayakta*: MarketDataRequest reddi, abonelik kaybı → gateway'i yeniden başlat (reconnect davranışı test edilmiş).
- *NATS lag artıyor*: yavaş consumer → ölçekle; JetStream disk doluluğu kontrol.
