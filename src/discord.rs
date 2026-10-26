// Discord Rich Presence over the local IPC socket (no extra dependency).
// Frames are [opcode u32 LE][length u32 LE][JSON]; opcode 0 = handshake, 1 = command.
use serde_json::json;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, PartialEq)]
pub struct Presence {
    pub client_id: String,
    pub details: String,
    pub state: String,
}

fn sockets() -> Vec<std::path::PathBuf> {
    let run = std::env::var("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| "/tmp".into());
    let mut v = vec![];
    for i in 0..10 {
        let n = format!("discord-ipc-{i}");
        // Native, Flatpak and Snap installs put the socket in different places.
        for sub in ["", "app/com.discordapp.Discord/", "snap.discord/", ".flatpak/dev.vencord.Vesktop/xdg-run/"] {
            v.push(run.join(format!("{sub}{n}")));
        }
    }
    v
}

fn frame(s: &mut UnixStream, op: u32, payload: &serde_json::Value) -> std::io::Result<()> {
    let b = payload.to_string().into_bytes();
    let mut buf = Vec::with_capacity(8 + b.len());
    buf.extend(op.to_le_bytes());
    buf.extend((b.len() as u32).to_le_bytes());
    buf.extend(b);
    s.write_all(&buf)?;
    // Read (and ignore) the reply so the socket buffer never fills up.
    let mut head = [0u8; 8];
    s.read_exact(&mut head)?;
    let len = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
    let mut rest = vec![0u8; len.min(1 << 20)];
    s.read_exact(&mut rest)
}

fn connect(client_id: &str) -> Option<UnixStream> {
    for p in sockets() {
        if let Ok(mut s) = UnixStream::connect(&p) {
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            if frame(&mut s, 0, &json!({"v": 1, "client_id": client_id})).is_ok() {
                return Some(s);
            }
        }
    }
    None
}

fn set(s: &mut UnixStream, p: Option<&Presence>, start: u64) -> std::io::Result<()> {
    let activity = p.map(|p| {
        json!({
            "details": p.details, "state": p.state,
            "timestamps": {"start": start},
            "assets": {"large_image": "skinvas", "large_text": "Skinvas"}
        })
    });
    frame(s, 1, &json!({"cmd": "SET_ACTIVITY", "args": {"pid": std::process::id(), "activity": activity}, "nonce": format!("{start}{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis())}))
}

/// Background worker; send `Some(presence)` to show, `None` to clear.
pub fn start() -> Sender<Option<Presence>> {
    let (tx, rx): (Sender<Option<Presence>>, Receiver<Option<Presence>>) = channel();
    std::thread::spawn(move || {
        let start = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let (mut cur, mut sock, mut sock_id): (Option<Presence>, Option<UnixStream>, String) = (None, None, String::new());
        loop {
            match rx.recv_timeout(Duration::from_secs(15)) {
                Ok(p) => cur = p,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            let Some(p) = cur.clone().filter(|p| !p.client_id.is_empty()) else {
                if let Some(s) = sock.as_mut() {
                    let _ = set(s, None, start);
                }
                continue;
            };
            if sock.is_none() || sock_id != p.client_id {
                sock = connect(&p.client_id);
                sock_id = p.client_id.clone();
            }
            // On any error drop the socket; the 15 s timeout retries (Discord may start later).
            if let Some(s) = sock.as_mut() {
                if set(s, Some(&p), start).is_err() {
                    sock = None;
                }
            }
        }
    });
    tx
}
