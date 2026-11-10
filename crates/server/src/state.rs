use amiibo_core::Library;
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use switch_pro::nfc::Scan;
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub enum LinkState {
    Idle,
    Preparing,
    Pairing,
    Connected,
    Initialized,
    Ready,
    Disconnected,
    Reconnecting,
    Error,
}
#[derive(Clone, PartialEq, Serialize)]
pub struct Status {
    pub link: LinkState,
    pub peer: Option<String>,
    pub report: Option<u8>,
    pub mtu: Option<usize>,
    pub power: u8,
    pub scan: Scan,
    pub armed_name: Option<String>,
    pub error: Option<String>,
    pub simulation: bool,
}
#[derive(Clone, Serialize)]
pub struct Log {
    pub time: u64,
    pub message: String,
}
pub struct App {
    pub token: String,
    pub simulation: bool,
    pub bind: String,
    pub db: Mutex<Library>,
    pub status: Mutex<Status>,
    pub logs: Mutex<VecDeque<Log>>,
    pub version: watch::Sender<u64>,
    pub tx: mpsc::Sender<Request>,
    pub closing: std::sync::atomic::AtomicBool,
    pub debug: std::sync::atomic::AtomicBool,
    pub diagnostics: serde_json::Value,
}
pub type Shared = Arc<App>;
pub enum Action {
    Pair,
    Reconnect,
    Disconnect,
    Press(String),
    Arm,
    Cancel,
    Replay,
}
pub struct Request {
    pub action: Action,
    pub response: oneshot::Sender<Result<(), String>>,
}
impl App {
    pub fn changed(&self) {
        self.version.send_modify(|n| *n = n.wrapping_add(1));
    }
    pub fn log(&self, message: impl Into<String>) {
        let message = message.into();
        eprintln!("{message}");
        let mut logs = self.logs.lock().unwrap();
        if logs.len() == 160 {
            logs.pop_front();
        }
        logs.push_back(Log {
            time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            message,
        });
        drop(logs);
        self.changed();
    }
    pub fn modify(&self, f: impl FnOnce(&mut Status)) {
        let mut state = self.status.lock().unwrap();
        let old = state.clone();
        f(&mut state);
        if old != *state {
            let msg = format!(
                "{:?} · NFC {:?} · report {:?} · MCU {}",
                state.link, state.scan, state.report, state.power
            );
            drop(state);
            self.log(msg);
        }
    }
    pub fn phase(&self, phase: LinkState) {
        self.modify(|s| s.link = phase);
    }
    pub fn snapshot(&self) -> Result<serde_json::Value, amiibo_core::Error> {
        let db = self.db.lock().unwrap();
        Ok(
            serde_json::json!({"controller":self.status.lock().unwrap().clone(),"library":db.list()?,"history":db.history()?,"logs":self.logs.lock().unwrap().clone(),"system":self.diagnostics,"bind":self.bind,"debug":self.debug.load(std::sync::atomic::Ordering::Relaxed)}),
        )
    }
}
