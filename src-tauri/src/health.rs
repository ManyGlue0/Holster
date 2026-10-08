//! Running / not running, polled lightly: once at startup, on every manifest
//! change, and every couple of seconds only while the dock or settings are open.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

use crate::apps::{Manifest, Registry, Status};
use crate::process;

pub struct Health {
    reg: Arc<Registry>,
    client: reqwest::Client,
    active: AtomicBool,
    wake: Notify,
}

impl Health {
    pub fn new(reg: Arc<Registry>) -> Arc<Self> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(900))
            .build()
            .expect("http client");
        Arc::new(Self { reg, client, active: AtomicBool::new(false), wake: Notify::new() })
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                me.poll_all().await;
                if me.active.load(Ordering::SeqCst) {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(2500)) => {}
                        _ = me.wake.notified() => {}
                    }
                } else {
                    me.wake.notified().await;
                }
            }
        });
    }

    /// Dock or settings visible: poll regularly while true.
    pub fn set_active(&self, on: bool) {
        let was = self.active.swap(on, Ordering::SeqCst);
        if on && !was {
            self.wake.notify_one();
        }
    }

    pub fn poke(&self) {
        self.wake.notify_one();
    }

    pub async fn check(&self, m: &Manifest) -> Status {
        if let Some(url) = &m.health {
            return match self.client.get(url).send().await {
                Ok(r) if r.status().is_success() => Status::Running,
                _ => Status::Stopped,
            };
        }
        if let Some(l) = &m.launch {
            let exe = l.exe.clone();
            let running = tokio::task::spawn_blocking(move || !process::pids_for(&exe).is_empty()).await.unwrap_or(false);
            return if running { Status::Running } else { Status::Stopped };
        }
        if let Some(p) = &m.preview {
            return match self.client.get(&p.url).send().await {
                Ok(r) if r.status().is_success() => Status::Running,
                _ => Status::Stopped,
            };
        }
        Status::Unknown
    }

    async fn poll_all(self: &Arc<Self>) {
        let mut set = tokio::task::JoinSet::new();
        for m in self.reg.entries().into_iter().filter_map(|e| e.manifest.ok()) {
            let me = self.clone();
            set.spawn(async move { (m.id.clone(), me.check(&m).await) });
        }
        let mut results = HashMap::new();
        while let Some(r) = set.join_next().await {
            if let Ok((id, st)) = r {
                results.insert(id, st);
            }
        }
        self.reg.set_statuses(results);
    }

    /// Waits until the app answers its health check (after a launch).
    pub async fn wait_running(&self, m: &Manifest, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.check(m).await == Status::Running {
                self.reg.set_status(&m.id, Status::Running);
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}
