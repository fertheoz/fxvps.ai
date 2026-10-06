//! core-engine: runs the [`oms::Engine`] on one dedicated thread (single
//! writer, LMAX-style). Commands arrive over a bounded channel, are stamped
//! with sequence number and timestamp, appended to a JSON-lines journal and
//! then applied. Startup restores the latest snapshot and replays the
//! journal tail. A small axum admin API exposes accounts and positions.
//!
//! Integration pieces:
//! - [`api`]: the [`api::CoreApi`] boundary used by client-gateway, with an
//!   in-process implementation ([`api::InProcessCore`]). A NATS-backed
//!   implementation is a follow-up.
//! - [`lp_fix`]: [`lp_fix::FixLpRouter`] (oms `LpRouter` over fix-gateway) and
//!   the bridge feeding LP quotes and executions back as journaled commands.
//! - [`output`]: turns engine events into account-level [`api::CoreEvent`]s.
//! - [`stack`]: starts fix-gateway + engine + bridge in-process (demo/dev).

pub mod admin;
pub mod replica;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use oms::{
    Command, Engine, EngineConfig, EngineSnapshot, Envelope, Event, NullRouter, RecordingRouter,
};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path as FsPath, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tokio::sync::oneshot;

pub mod api;
pub mod lp_agg;
pub mod lp_fix;
pub mod output;
pub mod stack;

pub type Query = Box<dyn FnOnce(&Engine) -> Value + Send>;

pub enum Request {
    Cmd(Box<Command>, oneshot::Sender<Vec<Event>>),
    Query(Query, oneshot::Sender<Value>),
    /// Typed read; the closure delivers its own result.
    Read(Box<dyn FnOnce(&Engine) + Send>),
    Shutdown,
}

/// Commands produced outside the writer in reaction to router calls (e.g.
/// an LP order that could not be sent); applied right after the command that
/// caused them, so they are journaled like everything else.
pub type LpFeedback = Arc<Mutex<VecDeque<Command>>>;

/// How A-book orders reach a liquidity provider.
pub enum LpMode {
    /// Fill immediately at the current LP quote (journaled simulator).
    Simulated,
    /// Real router (e.g. [`lp_fix::FixLpRouter`]); fills arrive later as
    /// `Command::LpFill` / `Command::LpReject`.
    External {
        router: Box<dyn oms::LpRouter>,
        feedback: LpFeedback,
    },
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub data_dir: PathBuf,
    pub snapshot_every: u64,
    pub channel_capacity: usize,
    pub engine: EngineConfig,
    /// Fill A-book orders immediately at the current LP quote (no real LP).
    pub simulate_lp: bool,
}

impl Settings {
    pub fn new(data_dir: impl Into<PathBuf>) -> Settings {
        Settings {
            data_dir: data_dir.into(),
            snapshot_every: 1_000,
            channel_capacity: 4_096,
            engine: EngineConfig::default(),
            simulate_lp: true,
        }
    }
}

/// Command latency samples of the writer thread (journal write + apply), µs.
#[derive(Default)]
pub struct LatencyStats {
    samples: Mutex<VecDeque<u32>>,
    total: std::sync::atomic::AtomicU64,
    started: std::sync::OnceLock<std::time::Instant>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencyReport {
    pub samples: usize,
    pub total_commands: u64,
    pub p50_us: u32,
    pub p95_us: u32,
    pub p99_us: u32,
    pub max_us: u32,
    pub uptime_s: u64,
}

impl LatencyStats {
    const CAP: usize = 20_000;

    fn record(&self, us: u32) {
        self.started.get_or_init(std::time::Instant::now);
        self.total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut s) = self.samples.lock() {
            if s.len() >= Self::CAP {
                s.pop_front();
            }
            s.push_back(us);
        }
    }

    pub fn report(&self) -> LatencyReport {
        let mut v: Vec<u32> = self
            .samples
            .lock()
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        v.sort_unstable();
        let pct = |p: f64| -> u32 {
            if v.is_empty() {
                0
            } else {
                v[((v.len() - 1) as f64 * p).round() as usize]
            }
        };
        LatencyReport {
            samples: v.len(),
            total_commands: self.total.load(std::sync::atomic::Ordering::Relaxed),
            p50_us: pct(0.5),
            p95_us: pct(0.95),
            p99_us: pct(0.99),
            max_us: v.last().copied().unwrap_or(0),
            uptime_s: self.started.get().map_or(0, |t| t.elapsed().as_secs()),
        }
    }
}

/// Cloneable handle used by async code to talk to the engine thread.
#[derive(Clone)]
pub struct EngineHandle {
    tx: SyncSender<Request>,
    stats: Arc<LatencyStats>,
}

impl EngineHandle {
    pub async fn command(&self, cmd: Command) -> Result<Vec<Event>, String> {
        let (t, r) = oneshot::channel();
        self.send(Request::Cmd(Box::new(cmd), t)).await?;
        r.await.map_err(|e| e.to_string())
    }

    /// Runs `f` on the writer thread and returns its typed result.
    pub async fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Engine) -> T + Send + 'static,
    ) -> Result<T, String> {
        let (t, r) = oneshot::channel();
        self.send(Request::Read(Box::new(move |e| {
            let _ = t.send(f(e));
        })))
        .await?;
        r.await.map_err(|e| e.to_string())
    }

    pub async fn query(
        &self,
        f: impl FnOnce(&Engine) -> Value + Send + 'static,
    ) -> Result<Value, String> {
        let (t, r) = oneshot::channel();
        self.send(Request::Query(Box::new(f), t)).await?;
        r.await.map_err(|e| e.to_string())
    }

    async fn send(&self, req: Request) -> Result<(), String> {
        let tx = self.tx.clone();
        // bounded channel: block a worker thread, not the async runtime
        tokio::task::spawn_blocking(move || tx.send(req).map_err(|e| e.to_string()))
            .await
            .map_err(|e| e.to_string())?
    }

    /// Blocking variant for non-async callers.
    pub fn command_blocking(&self, cmd: Command) -> Result<Vec<Event>, String> {
        let (t, r) = oneshot::channel();
        self.tx
            .send(Request::Cmd(Box::new(cmd), t))
            .map_err(|e| e.to_string())?;
        r.blocking_recv().map_err(|e| e.to_string())
    }

    pub fn query_blocking(
        &self,
        f: impl FnOnce(&Engine) -> Value + Send + 'static,
    ) -> Result<Value, String> {
        let (t, r) = oneshot::channel();
        self.tx
            .send(Request::Query(Box::new(f), t))
            .map_err(|e| e.to_string())?;
        r.blocking_recv().map_err(|e| e.to_string())
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(Request::Shutdown);
    }

    /// Writer-thread command latency (journal write + apply).
    pub fn latency(&self) -> LatencyReport {
        self.stats.report()
    }
}

fn journal_path(dir: &FsPath) -> PathBuf {
    dir.join("journal.jsonl")
}
fn snapshot_path(dir: &FsPath) -> PathBuf {
    dir.join("snapshot.json")
}

/// Restores state from snapshot + journal tail (no LP side effects).
pub fn recover(settings: &Settings) -> std::io::Result<(Engine, u64)> {
    fs::create_dir_all(&settings.data_dir)?;
    let snap: Option<EngineSnapshot> = fs::read(snapshot_path(&settings.data_dir))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let mut engine = match &snap {
        Some(s) => Engine::restore(s, Box::new(NullRouter)).map_err(std::io::Error::other)?,
        None => Engine::new(settings.engine.clone(), Box::new(NullRouter)),
    };
    let mut seq = engine.seq();
    if let Ok(f) = File::open(journal_path(&settings.data_dir)) {
        for line in BufReader::new(f).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            // a torn last line (crash mid-write) is ignored
            let Ok(env) = serde_json::from_str::<Envelope>(&line) else {
                break;
            };
            if env.seq > seq {
                engine.apply(&env);
                seq = env.seq;
            }
        }
    }
    Ok((engine, seq))
}

/// Result of [`verify`]: the state replayed from a data directory, with no LP
/// side effects. `ok` = replay succeeded and the engine invariants hold.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VerifyReport {
    pub ok: bool,
    pub seq: u64,
    pub digest: String,
    pub snapshot_seq: Option<u64>,
    pub journal_bytes: u64,
    pub journal_lines: u64,
    pub accounts: usize,
    pub positions: usize,
    pub error: Option<String>,
}

/// Replays `data_dir` (snapshot + journal) and checks the invariants — the
/// restore drill of a backup and the deploy gate of a new engine image
/// (`core-engine verify --data-dir DIR`). Read-only.
pub fn verify(data_dir: impl Into<PathBuf>) -> std::io::Result<VerifyReport> {
    let settings = Settings::new(data_dir);
    let snapshot_seq = fs::read(snapshot_path(&settings.data_dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<EngineSnapshot>(&b).ok())
        .map(|s| s.seq());
    let (journal_bytes, journal_lines) = match File::open(journal_path(&settings.data_dir)) {
        Ok(f) => {
            let bytes = f.metadata().map(|m| m.len()).unwrap_or(0);
            let lines = BufReader::new(f)
                .lines()
                .map_while(Result::ok)
                .filter(|l| !l.trim().is_empty())
                .count() as u64;
            (bytes, lines)
        }
        Err(_) => (0, 0),
    };
    let (engine, seq) = recover(&settings)?;
    let error = engine.check_invariants().err();
    let positions = engine
        .accounts()
        .map(|a| engine.positions_of(a.id).len())
        .sum();
    Ok(VerifyReport {
        ok: error.is_none(),
        seq,
        digest: short_digest(&engine),
        snapshot_seq,
        journal_bytes,
        journal_lines,
        accounts: engine.accounts().count(),
        positions,
        error,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CompactReport {
    pub seq: u64,
    pub digest: String,
    /// Where the old journal went (`None` when it was already empty).
    pub archived: Option<PathBuf>,
    pub archived_bytes: u64,
}

/// Writes a fresh snapshot of `engine` and rotates the journal into
/// `archive/journal-<seq>.jsonl`. Only valid while no writer has the journal open.
fn compact_with(engine: &Engine, seq: u64, dir: &FsPath) -> std::io::Result<CompactReport> {
    let snap = engine.snapshot();
    let tmp = dir.join("snapshot.json.tmp");
    fs::write(
        &tmp,
        serde_json::to_vec(&snap).map_err(std::io::Error::other)?,
    )?;
    fs::rename(tmp, snapshot_path(dir))?;
    let jp = journal_path(dir);
    let bytes = fs::metadata(&jp).map(|m| m.len()).unwrap_or(0);
    let archived = if bytes > 0 {
        let arch = dir.join("archive");
        fs::create_dir_all(&arch)?;
        let target = arch.join(format!("journal-{seq}.jsonl"));
        fs::rename(&jp, &target)?;
        Some(target)
    } else {
        None
    };
    File::create(&jp)?;
    Ok(CompactReport {
        seq,
        digest: short_digest(engine),
        archived,
        archived_bytes: bytes,
    })
}

/// Snapshot + journal rotation of a data directory nobody is writing to
/// (`core-engine compact --data-dir DIR`). State is unchanged: replaying the
/// compacted directory yields the same digest.
pub fn compact(data_dir: impl Into<PathBuf>) -> std::io::Result<CompactReport> {
    let settings = Settings::new(data_dir);
    let (engine, seq) = recover(&settings)?;
    engine.check_invariants().map_err(std::io::Error::other)?;
    compact_with(&engine, seq, &settings.data_dir)
}

/// 16-hex-digit fingerprint of the full state (the raw digest is the whole
/// state as JSON: megabytes). Equal states → equal fingerprints.
fn short_digest(engine: &Engine) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    engine.state_digest().hash(&mut h);
    format!("{:016x}", h.finish())
}

fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

struct Writer {
    engine: Engine,
    seq: u64,
    last_ts: u64,
    journal: File,
    settings: Settings,
    lp: WriterLp,
    since_snapshot: u64,
    publisher: Option<output::Publisher>,
    stats: Arc<LatencyStats>,
}

enum WriterLp {
    Simulated(RecordingRouter),
    External(LpFeedback),
}

impl Writer {
    fn apply(&mut self, cmd: Command) -> std::io::Result<Vec<Event>> {
        let started = std::time::Instant::now();
        self.seq += 1;
        self.last_ts = now_ns().max(self.last_ts);
        let env = Envelope {
            seq: self.seq,
            ts: self.last_ts,
            cmd,
        };
        let mut line = serde_json::to_vec(&env).map_err(std::io::Error::other)?;
        line.push(b'\n');
        self.journal.write_all(&line)?;
        let ev = self.engine.apply(&env);
        self.stats
            .record(started.elapsed().as_micros().min(u32::MAX as u128) as u32);
        self.since_snapshot += 1;
        if self.since_snapshot >= self.settings.snapshot_every {
            self.snapshot()?;
        }
        Ok(ev)
    }

    fn snapshot(&mut self) -> std::io::Result<()> {
        self.journal.flush()?;
        self.journal.sync_data()?;
        let snap = self.engine.snapshot();
        let tmp = self.settings.data_dir.join("snapshot.json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec(&snap).map_err(std::io::Error::other)?,
        )?;
        fs::rename(tmp, snapshot_path(&self.settings.data_dir))?;
        self.since_snapshot = 0;
        Ok(())
    }

    /// Commands caused by the previous one (simulated LP fills).
    fn internal_commands(&mut self) -> VecDeque<Command> {
        let mut q = VecDeque::new();
        let router = match &self.lp {
            WriterLp::Simulated(r) => r,
            WriterLp::External(fb) => {
                if let Ok(mut fb) = fb.lock() {
                    q.extend(fb.drain(..));
                }
                return q;
            }
        };
        for r in router.take() {
            let quote = self.engine.snapshot_quote(&r.symbol);
            match quote {
                Some(qt) => q.push_back(Command::LpFill {
                    lp_order_id: r.lp_order_id,
                    exec_id: format!("sim-{}", r.lp_order_id),
                    volume: r.volume,
                    price: qt.for_side(r.side),
                }),
                None => q.push_back(Command::LpReject {
                    lp_order_id: r.lp_order_id,
                    reason: "no quote".into(),
                }),
            }
        }
        q
    }

    fn run(mut self, rx: Receiver<Request>) {
        while let Ok(req) = rx.recv() {
            match req {
                Request::Cmd(cmd, reply) => {
                    let mut events = Vec::new();
                    let mut queue = VecDeque::from([*cmd]);
                    while let Some(c) = queue.pop_front() {
                        let quote_sym = match &c {
                            Command::Quote { symbol, .. } => Some(symbol.clone()),
                            _ => None,
                        };
                        match self.apply(c) {
                            Ok(ev) => {
                                if let Some(p) = &mut self.publisher {
                                    p.publish(
                                        &self.engine,
                                        quote_sym.as_deref(),
                                        &ev,
                                        self.last_ts,
                                    );
                                }
                                events.extend(ev)
                            }
                            Err(e) => {
                                tracing::error!("journal write failed: {e}");
                                events.push(Event::CommandRejected {
                                    reason: format!("journal: {e}"),
                                });
                                break;
                            }
                        }
                        queue.extend(self.internal_commands());
                    }
                    let _ = reply.send(events);
                }
                Request::Query(f, reply) => {
                    let _ = reply.send(f(&self.engine));
                }
                Request::Read(f) => f(&self.engine),
                Request::Shutdown => break,
            }
        }
        if let Err(e) = self.snapshot() {
            tracing::error!("final snapshot failed: {e}");
        }
    }
}

/// Recovers state and starts the single writer thread. A-book orders are
/// filled by the journaled simulator when `settings.simulate_lp`, otherwise
/// they are dropped (no LP attached).
pub fn spawn(settings: Settings) -> std::io::Result<(EngineHandle, JoinHandle<()>)> {
    let lp = if settings.simulate_lp {
        LpMode::Simulated
    } else {
        LpMode::External {
            router: Box::new(NullRouter),
            feedback: LpFeedback::default(),
        }
    };
    spawn_with(settings, lp, None)
}

/// Like [`spawn`] with an explicit LP mode and an optional account-event
/// publisher (see [`output::Publisher`]).
pub fn spawn_with(
    settings: Settings,
    lp: LpMode,
    publisher: Option<output::Publisher>,
) -> std::io::Result<(EngineHandle, JoinHandle<()>)> {
    let (mut engine, seq) = recover(&settings)?;
    let lp = match lp {
        LpMode::Simulated => {
            let router = RecordingRouter::default();
            engine.set_router(Box::new(router.clone()));
            WriterLp::Simulated(router)
        }
        LpMode::External { router, feedback } => {
            engine.set_router(router);
            WriterLp::External(feedback)
        }
    };
    // Journals grow without bound; above `CORE_JOURNAL_COMPACT_MB` (default 256,
    // 0 = never) the start-up snapshots and rotates the old journal into archive/.
    let limit_mb: u64 = std::env::var("CORE_JOURNAL_COMPACT_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);
    if limit_mb > 0 {
        let bytes = fs::metadata(journal_path(&settings.data_dir))
            .map(|m| m.len())
            .unwrap_or(0);
        if bytes > limit_mb * 1024 * 1024 {
            let r = compact_with(&engine, seq, &settings.data_dir)?;
            tracing::info!(seq, archived = ?r.archived, bytes, "journal compacted at start-up");
        }
    }
    let journal = OpenOptions::new()
        .create(true)
        .append(true)
        .open(journal_path(&settings.data_dir))?;
    let (tx, rx) = sync_channel(settings.channel_capacity);
    let stats = Arc::new(LatencyStats::default());
    let writer = Writer {
        engine,
        seq,
        last_ts: 0,
        journal,
        settings,
        lp,
        since_snapshot: 0,
        publisher,
        stats: stats.clone(),
    };
    let join = std::thread::Builder::new()
        .name("core-engine-writer".into())
        .spawn(move || writer.run(rx))?;
    Ok((EngineHandle { tx, stats }, join))
}

// ----------------------------------------------------------------------
// Admin HTTP API
// ----------------------------------------------------------------------

fn account_json(e: &Engine, id: u64) -> Value {
    let Some(a) = e.account(id) else {
        return Value::Null;
    };
    let risk = e.account_risk(id).ok();
    json!({ "account": a, "risk": risk, "positions": e.positions_of(id).len() })
}

pub fn router(h: EngineHandle) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/accounts", get(list_accounts))
        .route("/accounts/{id}", get(get_account))
        .route("/accounts/{id}/positions", get(get_positions))
        .route("/commands", post(post_command))
        .with_state(h)
}

type ApiResult = Result<Json<Value>, (StatusCode, String)>;

fn internal(e: String) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e)
}

async fn list_accounts(State(h): State<EngineHandle>) -> ApiResult {
    h.query(|e| Value::Array(e.accounts().map(|a| account_json(e, a.id)).collect()))
        .await
        .map(Json)
        .map_err(internal)
}

async fn get_account(State(h): State<EngineHandle>, Path(id): Path<u64>) -> ApiResult {
    let v = h
        .query(move |e| account_json(e, id))
        .await
        .map_err(internal)?;
    if v.is_null() {
        return Err((StatusCode::NOT_FOUND, "unknown account".into()));
    }
    Ok(Json(v))
}

async fn get_positions(State(h): State<EngineHandle>, Path(id): Path<u64>) -> ApiResult {
    h.query(move |e| json!(e.positions_of(id)))
        .await
        .map(Json)
        .map_err(internal)
}

async fn post_command(State(h): State<EngineHandle>, Json(cmd): Json<Command>) -> ApiResult {
    h.command(cmd)
        .await
        .map(|ev| Json(json!(ev)))
        .map_err(internal)
}
