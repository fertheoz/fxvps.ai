//! Read replica for heavy reports (stage 14): a second [`Engine`] rebuilt
//! from the snapshot and kept current by tailing the journal file, so report
//! queries never queue behind trading commands on the writer thread. It is a
//! few hundred milliseconds behind at most (one refresh before each query).
//! A journal rotation (compaction) or any gap re-synchronises from the
//! snapshot.

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;

use oms::{Engine, Envelope, NullRouter};

use crate::{recover, Settings};

pub struct Replica {
    settings: Settings,
    engine: Engine,
    seq: u64,
    /// Byte offset in `journal.jsonl` up to which lines were applied.
    offset: u64,
    pub reloads: u64,
    pub applied: u64,
}

impl Replica {
    pub fn open(data_dir: impl Into<PathBuf>) -> std::io::Result<Replica> {
        let settings = Settings::new(data_dir);
        let (engine, seq) = recover(&settings)?;
        let offset = std::fs::metadata(settings.data_dir.join("journal.jsonl"))
            .map(|m| m.len())
            .unwrap_or(0);
        Ok(Replica {
            settings,
            engine,
            seq,
            offset,
            reloads: 0,
            applied: 0,
        })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    fn reload(&mut self) -> std::io::Result<()> {
        let (mut engine, seq) = recover(&self.settings)?;
        engine.set_router(Box::new(NullRouter));
        self.engine = engine;
        self.seq = seq;
        self.offset = std::fs::metadata(self.settings.data_dir.join("journal.jsonl"))
            .map(|m| m.len())
            .unwrap_or(0);
        self.reloads += 1;
        Ok(())
    }

    /// Applies the journal lines written since the last call; returns how many.
    pub fn refresh(&mut self) -> std::io::Result<usize> {
        let path = self.settings.data_dir.join("journal.jsonl");
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if len < self.offset {
            // rotated (compaction) → start over from the snapshot
            self.reload()?;
            return Ok(0);
        }
        if len == self.offset {
            return Ok(0);
        }
        let mut f = File::open(&path)?;
        f.seek(SeekFrom::Start(self.offset))?;
        let mut reader = BufReader::new(f);
        let mut line = String::new();
        let mut n = 0usize;
        loop {
            line.clear();
            let read = reader.read_line(&mut line)?;
            if read == 0 {
                break;
            }
            if !line.ends_with('\n') {
                break; // torn tail: wait for the writer to finish the line
            }
            let Ok(env) = serde_json::from_str::<Envelope>(line.trim_end()) else {
                break;
            };
            if env.seq > self.seq + 1 {
                // gap: the journal was rotated and this file starts later than we are
                self.reload()?;
                return Ok(0);
            }
            if env.seq == self.seq + 1 {
                self.engine.apply(&env);
                self.seq = env.seq;
                self.applied += 1;
                n += 1;
            }
            self.offset += read as u64;
        }
        Ok(n)
    }
}
