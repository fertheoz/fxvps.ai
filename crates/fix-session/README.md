# fix-session

FIX 4.4 session layer.

- `Session` — sans-IO state machine. Inputs: `on_connect`, `on_frame`, `on_timer`,
  `send_app`, `logout` (time is passed in, so tests are deterministic). Output: `Action`s
  (`Send`, `Deliver`, `PeerReject`, `LoggedOn`, `Disconnect`).
  - Logon (initiator/acceptor, optional credential check, ResetSeqNumFlag)
  - Heartbeat when idle for HeartBtInt; TestRequest after 1.2 × HeartBtInt of inbound
    silence; disconnect if no reply within another HeartBtInt
  - Inbound seq checks: gap → queue + ResendRequest(expected, 0) → in-order delivery once filled;
    too low without PossDup → Logout; PossDup duplicates ignored
  - Answering ResendRequest: application messages re-sent with PossDup=Y + OrigSendingTime,
    admin messages / missing journal entries collapsed into SequenceReset-GapFill
  - SequenceReset (gap-fill and reset), session Reject for invalid bodies (seq consumed),
    Reject for unsupported MsgType, graceful Logout with timeout
- `MessageStore` — `MemoryStore`, `FileStore` (seqnums file replaced atomically + append-only
  journal; torn tail records are ignored on open). fsync policy is a follow-up.
- `run_session` — tokio driver over any `AsyncRead + AsyncWrite`; `tls` feature adds
  `tls::connect_tls` (rustls).
