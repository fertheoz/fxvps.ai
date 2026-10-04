//! Persistent sequence numbers and outbound message journal (for resends).

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};

/// Storage for session sequence numbers and sent application messages.
pub trait MessageStore: Send {
    fn next_sender_seq(&self) -> u64;
    fn next_target_seq(&self) -> u64;
    fn set_next_sender_seq(&mut self, seq: u64) -> io::Result<()>;
    fn set_next_target_seq(&mut self, seq: u64) -> io::Result<()>;
    /// Stores an outbound message (full encoded frame) under its sequence number.
    fn store(&mut self, seq: u64, msg: &[u8]) -> io::Result<()>;
    /// Returns stored messages with `begin <= seq <= end`, ascending.
    fn get(&self, begin: u64, end: u64) -> io::Result<Vec<(u64, Vec<u8>)>>;
    /// Resets both sequence numbers to 1 and drops the journal.
    fn reset(&mut self) -> io::Result<()>;
}

impl<T: MessageStore + ?Sized> MessageStore for Box<T> {
    fn next_sender_seq(&self) -> u64 {
        (**self).next_sender_seq()
    }
    fn next_target_seq(&self) -> u64 {
        (**self).next_target_seq()
    }
    fn set_next_sender_seq(&mut self, seq: u64) -> io::Result<()> {
        (**self).set_next_sender_seq(seq)
    }
    fn set_next_target_seq(&mut self, seq: u64) -> io::Result<()> {
        (**self).set_next_target_seq(seq)
    }
    fn store(&mut self, seq: u64, msg: &[u8]) -> io::Result<()> {
        (**self).store(seq, msg)
    }
    fn get(&self, begin: u64, end: u64) -> io::Result<Vec<(u64, Vec<u8>)>> {
        (**self).get(begin, end)
    }
    fn reset(&mut self) -> io::Result<()> {
        (**self).reset()
    }
}

/// Volatile store (tests, simulators, sessions that reset on every logon).
#[derive(Debug, Clone)]
pub struct MemoryStore {
    sender: u64,
    target: u64,
    messages: BTreeMap<u64, Vec<u8>>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        MemoryStore {
            sender: 1,
            target: 1,
            messages: BTreeMap::new(),
        }
    }
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl MessageStore for MemoryStore {
    fn next_sender_seq(&self) -> u64 {
        self.sender
    }
    fn next_target_seq(&self) -> u64 {
        self.target
    }
    fn set_next_sender_seq(&mut self, seq: u64) -> io::Result<()> {
        self.sender = seq;
        Ok(())
    }
    fn set_next_target_seq(&mut self, seq: u64) -> io::Result<()> {
        self.target = seq;
        Ok(())
    }
    fn store(&mut self, seq: u64, msg: &[u8]) -> io::Result<()> {
        self.messages.insert(seq, msg.to_vec());
        Ok(())
    }
    fn get(&self, begin: u64, end: u64) -> io::Result<Vec<(u64, Vec<u8>)>> {
        if begin > end {
            return Ok(Vec::new());
        }
        Ok(self
            .messages
            .range(begin..=end)
            .map(|(k, v)| (*k, v.clone()))
            .collect())
    }
    fn reset(&mut self) -> io::Result<()> {
        *self = MemoryStore::default();
        Ok(())
    }
}

/// File-backed store: `seqnums` (atomically replaced) + append-only `messages.log`
/// with records `[seq: u64 LE][len: u32 LE][bytes]`. The journal is indexed in memory
/// on open. Each write is flushed (fsync is left to the OS for M1; see README).
#[derive(Debug)]
pub struct FileStore {
    dir: PathBuf,
    mem: MemoryStore,
    log: File,
}

impl FileStore {
    pub fn open(dir: impl AsRef<Path>) -> io::Result<FileStore> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        let mut mem = MemoryStore::default();
        match fs::read_to_string(dir.join("seqnums")) {
            Ok(s) => {
                let mut it = s.split_whitespace().map(str::parse::<u64>);
                match (it.next(), it.next()) {
                    (Some(Ok(a)), Some(Ok(b))) => {
                        mem.sender = a;
                        mem.target = b;
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "corrupt seqnums",
                        ))
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let log_path = dir.join("messages.log");
        if log_path.exists() {
            let mut r = BufReader::new(File::open(&log_path)?);
            let mut head = [0u8; 12];
            loop {
                match r.read_exact(&mut head) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                    Err(e) => return Err(e),
                }
                let mut seq_b = [0u8; 8];
                seq_b.copy_from_slice(&head[..8]);
                let mut len_b = [0u8; 4];
                len_b.copy_from_slice(&head[8..]);
                let mut buf = vec![0u8; u32::from_le_bytes(len_b) as usize];
                if r.read_exact(&mut buf).is_err() {
                    break; // torn tail write; ignore the partial record
                }
                mem.messages.insert(u64::from_le_bytes(seq_b), buf);
            }
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        Ok(FileStore { dir, mem, log })
    }

    fn persist_seqnums(&self) -> io::Result<()> {
        let tmp = self.dir.join("seqnums.tmp");
        fs::write(&tmp, format!("{} {}\n", self.mem.sender, self.mem.target))?;
        fs::rename(tmp, self.dir.join("seqnums"))
    }
}

impl MessageStore for FileStore {
    fn next_sender_seq(&self) -> u64 {
        self.mem.sender
    }
    fn next_target_seq(&self) -> u64 {
        self.mem.target
    }
    fn set_next_sender_seq(&mut self, seq: u64) -> io::Result<()> {
        self.mem.sender = seq;
        self.persist_seqnums()
    }
    fn set_next_target_seq(&mut self, seq: u64) -> io::Result<()> {
        self.mem.target = seq;
        self.persist_seqnums()
    }
    fn store(&mut self, seq: u64, msg: &[u8]) -> io::Result<()> {
        let len = u32::try_from(msg.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "message too large"))?;
        let mut rec = Vec::with_capacity(12 + msg.len());
        rec.extend_from_slice(&seq.to_le_bytes());
        rec.extend_from_slice(&len.to_le_bytes());
        rec.extend_from_slice(msg);
        self.log.write_all(&rec)?;
        self.log.flush()?;
        self.mem.messages.insert(seq, msg.to_vec());
        Ok(())
    }
    fn get(&self, begin: u64, end: u64) -> io::Result<Vec<(u64, Vec<u8>)>> {
        self.mem.get(begin, end)
    }
    fn reset(&mut self) -> io::Result<()> {
        self.mem = MemoryStore::default();
        self.log = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(self.dir.join("messages.log"))?;
        self.persist_seqnums()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut s = FileStore::open(dir.path()).unwrap();
            assert_eq!((s.next_sender_seq(), s.next_target_seq()), (1, 1));
            s.store(1, b"one").unwrap();
            s.store(2, b"two").unwrap();
            s.set_next_sender_seq(3).unwrap();
            s.set_next_target_seq(7).unwrap();
        }
        let mut s = FileStore::open(dir.path()).unwrap();
        assert_eq!((s.next_sender_seq(), s.next_target_seq()), (3, 7));
        assert_eq!(
            s.get(1, 5).unwrap(),
            vec![(1, b"one".to_vec()), (2, b"two".to_vec())]
        );
        assert_eq!(s.get(2, 2).unwrap().len(), 1);
        s.reset().unwrap();
        drop(s);
        let s = FileStore::open(dir.path()).unwrap();
        assert_eq!((s.next_sender_seq(), s.next_target_seq()), (1, 1));
        assert!(s.get(1, 100).unwrap().is_empty());
    }

    #[test]
    fn file_store_ignores_torn_tail() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut s = FileStore::open(dir.path()).unwrap();
            s.store(1, b"ok").unwrap();
        }
        let mut f = OpenOptions::new()
            .append(true)
            .open(dir.path().join("messages.log"))
            .unwrap();
        f.write_all(&5u64.to_le_bytes()).unwrap();
        f.write_all(&100u32.to_le_bytes()).unwrap();
        f.write_all(b"short").unwrap();
        let s = FileStore::open(dir.path()).unwrap();
        assert_eq!(s.get(1, 10).unwrap(), vec![(1, b"ok".to_vec())]);
    }
}
