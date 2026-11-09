//! Settings-process view of the tray's service.
//!
//! Exposes the same `snapshot()` / `submit()` surface the UI already uses, but
//! backed by IPC to the background tray. A local copy of the last snapshot is
//! kept so rendering never blocks on the pipe: a background thread polls the
//! tray's cached snapshot (cheap — the tray never touches hardware to answer).
use crate::ipc::Client;
use crate::service::{Action, Snapshot};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub struct Service {
    latest: Arc<Mutex<Snapshot>>,
    actions: Sender<Action>,
}

impl Service {
    /// Connects to the tray and loads its cached snapshot before the window
    /// opens, so the first frame already shows real data.
    pub fn connect() -> anyhow::Result<Self> {
        let mut client = Client::connect()?;
        let latest = Arc::new(Mutex::new(client.snapshot()?));
        let (actions, receiver) = mpsc::channel::<Action>();
        let shared = latest.clone();
        std::thread::spawn(move || loop {
            // Forward queued actions first so the UI sees "busy" promptly.
            while let Ok(action) = receiver.try_recv() {
                if client.submit(action).is_err() {
                    return;
                }
            }
            match client.snapshot() {
                Ok(snapshot) => *shared.lock().unwrap_or_else(|e| e.into_inner()) = snapshot,
                Err(_) => return,
            }
            // Wake early for a pending action instead of a fixed sleep.
            match receiver.recv_timeout(Duration::from_millis(250)) {
                Ok(action) => {
                    if client.submit(action).is_err() {
                        return;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        });
        Ok(Self { latest, actions })
    }

    /// Last snapshot received from the tray. Never blocks on IPC.
    pub fn snapshot(&self) -> Snapshot {
        self.latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn submit(&self, action: Action) {
        let _ = self.actions.send(action);
    }
}
