//! Tokio transport: drives a [`Session`] over any `AsyncRead + AsyncWrite` stream.

use std::time::Duration;

use fix_codec::{frame_len, Body, Message, Reject};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{debug, warn};

use crate::session::{Action, Session};
use crate::store::MessageStore;
use crate::SessionError;

/// Commands from the application to the session task.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // commands are moved once through a channel
pub enum SessionCommand {
    Send(Body),
    Logout(Option<String>),
}

/// Events from the session task to the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    LoggedOn,
    App(Box<Message>),
    PeerReject(Reject),
    /// Final event; the session task ends right after.
    Disconnected(String),
}

/// Timer granularity for heartbeat/timeout checks.
const TICK: Duration = Duration::from_millis(100);

/// Runs the session until disconnect. Returns the session (and its store) so the
/// caller can reconnect while keeping sequence numbers.
pub async fn run_session<T, S>(
    mut io: T,
    mut session: Session<S>,
    mut commands: mpsc::Receiver<SessionCommand>,
    events: mpsc::Sender<SessionEvent>,
) -> Session<S>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: MessageStore,
{
    let reason = drive(&mut io, &mut session, &mut commands, &events).await;
    let reason = match reason {
        Ok(r) => r,
        Err(e) => e.to_string(),
    };
    debug!(%reason, "session ended");
    let _ = io.shutdown().await;
    let _ = events.send(SessionEvent::Disconnected(reason)).await;
    session
}

async fn drive<T, S>(
    io: &mut T,
    session: &mut Session<S>,
    commands: &mut mpsc::Receiver<SessionCommand>,
    events: &mpsc::Sender<SessionEvent>,
) -> Result<String, SessionError>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: MessageStore,
{
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut chunk = vec![0u8; 64 * 1024];
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut commands_open = true;

    let actions = session.on_connect(Instant::now().into_std())?;
    if let Some(r) = apply(io, events, actions).await? {
        return Ok(r);
    }

    loop {
        let actions = tokio::select! {
            n = io.read(&mut chunk) => {
                let n = n?;
                if n == 0 {
                    return Ok("connection closed by peer".into());
                }
                buf.extend_from_slice(&chunk[..n]);
                let mut actions = Vec::new();
                let mut consumed = 0;
                loop {
                    match frame_len(&buf[consumed..]) {
                        Ok(Some(len)) => {
                            let frame = &buf[consumed..consumed + len];
                            actions.extend(session.on_frame(frame, Instant::now().into_std())?);
                            consumed += len;
                        }
                        Ok(None) => break,
                        Err(e) => {
                            warn!(error = %e, "unrecoverable framing error");
                            return Err(SessionError::Framing(e));
                        }
                    }
                }
                buf.drain(..consumed);
                actions
            }
            cmd = commands.recv(), if commands_open => {
                let now = Instant::now().into_std();
                match cmd {
                    Some(SessionCommand::Send(body)) => match session.send_app(body, now) {
                        Ok(a) => a,
                        Err(e) => {
                            warn!(error = %e, "dropping outbound message");
                            Vec::new()
                        }
                    },
                    Some(SessionCommand::Logout(text)) => session.logout(text, now)?,
                    None => {
                        commands_open = false;
                        session.logout(Some("application closed".into()), now)?
                    }
                }
            }
            _ = ticker.tick() => session.on_timer(Instant::now().into_std())?,
        };
        if let Some(r) = apply(io, events, actions).await? {
            return Ok(r);
        }
    }
}

async fn apply<T: AsyncWrite + Unpin>(
    io: &mut T,
    events: &mpsc::Sender<SessionEvent>,
    actions: Vec<Action>,
) -> Result<Option<String>, SessionError> {
    let mut wrote = false;
    for a in actions {
        match a {
            Action::Send(bytes) => {
                io.write_all(&bytes).await?;
                wrote = true;
            }
            Action::Deliver(m) => {
                let _ = events.send(SessionEvent::App(m)).await;
            }
            Action::PeerReject(r) => {
                let _ = events.send(SessionEvent::PeerReject(r)).await;
            }
            Action::LoggedOn => {
                let _ = events.send(SessionEvent::LoggedOn).await;
            }
            Action::Disconnect(reason) => {
                if wrote {
                    io.flush().await?;
                }
                return Ok(Some(reason));
            }
        }
    }
    if wrote {
        io.flush().await?;
    }
    Ok(None)
}
