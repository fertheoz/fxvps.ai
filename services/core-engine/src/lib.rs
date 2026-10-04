//! core-engine: runs the [`oms::Engine`] on one dedicated thread (single
//! writer, LMAX-style). Commands arrive over a bounded channel, are stamped
//! with sequence number and timestamp, appended to a JSON-lines journal and
//! then applied. Startup restores the latest snapshot and replays the
//! journal tail. A small axum admin API exposes accounts and positions.

pub mod admin;

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
use std::thread::JoinHandle;
use tokio::sync::oneshot;

pub type Query = Box<dyn FnOnce(&Engine) -> Value + Send>;

pub enum Request {
    Cmd(Command, oneshot::Sender<Vec<Event>>),
    Query(Query, oneshot::Sender<Value>),
    Shutdown,
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

/// Cloneable handle used by async code to talk to the engine thread.
#[derive(Clone)]
pub struct EngineHandle {
    tx: SyncSender<Request>,
}

impl EngineHandle {
    pub async fn command(&self, cmd: Command) -> Result<Vec<Event>, String> {
        let (t, r) = oneshot::channel();
        self.send(Request::Cmd(cmd, t)).await?;
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
            .send(Request::Cmd(cmd, t))
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
    router: RecordingRouter,
    since_snapshot: u64,
}

impl Writer {
    fn apply(&mut self, cmd: Command) -> std::io::Result<Vec<Event>> {
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
        for r in self.router.take() {
            if !self.settings.simulate_lp {
                continue;
            }
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
                    let mut queue = VecDeque::from([cmd]);
                    while let Some(c) = queue.pop_front() {
                        match self.apply(c) {
                            Ok(ev) => events.extend(ev),
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
                Request::Shutdown => break,
            }
        }
        if let Err(e) = self.snapshot() {
            tracing::error!("final snapshot failed: {e}");
        }
    }
}

/// Recovers state and starts the single writer thread.
pub fn spawn(settings: Settings) -> std::io::Result<(EngineHandle, JoinHandle<()>)> {
    let (mut engine, seq) = recover(&settings)?;
    let router = RecordingRouter::default();
    engine.set_router(Box::new(router.clone()));
    let journal = OpenOptions::new()
        .create(true)
        .append(true)
        .open(journal_path(&settings.data_dir))?;
    let (tx, rx) = sync_channel(settings.channel_capacity);
    let writer = Writer {
        engine,
        seq,
        last_ts: 0,
        journal,
        settings,
        router,
        since_snapshot: 0,
    };
    let join = std::thread::Builder::new()
        .name("core-engine-writer".into())
        .spawn(move || writer.run(rx))?;
    Ok((EngineHandle { tx }, join))
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
