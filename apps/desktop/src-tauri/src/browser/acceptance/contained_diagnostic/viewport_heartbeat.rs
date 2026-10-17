//! A fixture observer must not stall the independent trusted viewport heartbeat.

use super::{BrowserRect, present};
use anyhow::{Context as _, ensure};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::{sync::Mutex, task::JoinHandle};

const PERIOD: Duration = Duration::from_millis(100);

#[derive(Clone)]
struct Pulse {
    tab: String,
    rect: BrowserRect,
}

enum Mode {
    Paused,
    Visible(Pulse),
    Stopped,
}

pub(super) struct ViewportHeartbeat {
    mode: Arc<Mutex<Mode>>,
    task: Option<JoinHandle<anyhow::Result<()>>>,
}

impl ViewportHeartbeat {
    pub(super) fn start(app: tauri::AppHandle) -> Self {
        Self::start_with(move |pulse| {
            let app = app.clone();
            async move { present(&app, &pulse.tab, pulse.rect).await }
        })
    }

    fn start_with<F, Fut>(renew: F) -> Self
    where
        F: Fn(Pulse) -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<()>> + Send,
    {
        let mode = Arc::new(Mutex::new(Mode::Paused));
        let owned = mode.clone();
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(PERIOD);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let mode = owned.lock().await;
                match &*mode {
                    Mode::Paused => {}
                    Mode::Stopped => return Ok(()),
                    Mode::Visible(pulse) => {
                        // Pause/stop wait for this complete native acknowledgement.
                        // Holding an async mutex also fences queued main-thread show.
                        renew(pulse.clone()).await?;
                    }
                }
            }
        });
        Self {
            mode,
            task: Some(task),
        }
    }

    pub(super) fn check(&self) -> anyhow::Result<()> {
        ensure!(
            self.task.as_ref().is_some_and(|task| !task.is_finished()),
            "trusted fixture viewport heartbeat ended before shutdown"
        );
        Ok(())
    }

    pub(super) async fn resume(&self, tab: &str, rect: BrowserRect) -> anyhow::Result<()> {
        self.check()?;
        let mut mode = self.mode.lock().await;
        ensure!(
            !matches!(*mode, Mode::Stopped),
            "viewport heartbeat already stopped"
        );
        *mode = Mode::Visible(Pulse {
            tab: tab.to_owned(),
            rect,
        });
        drop(mode);
        self.check()
    }

    pub(super) async fn pause(&self) -> anyhow::Result<()> {
        self.check()?;
        // Await the previous pulse before publishing the pause; callers hide only
        // after this returns. Subsequent ticks observe Paused and cannot show.
        *self.mode.lock().await = Mode::Paused;
        self.check()
    }

    pub(super) async fn stop(&mut self) -> anyhow::Result<()> {
        if self.task.is_none() {
            return Ok(());
        }
        *self.mode.lock().await = Mode::Stopped;
        let result = self.task.as_mut().expect("retained heartbeat task").await;
        self.task.take();
        result.context("viewport heartbeat task did not settle")?
    }
}

impl Drop for ViewportHeartbeat {
    fn drop(&mut self) {
        // Unexpected outer task cancellation drops the native acknowledgement
        // receiver, which the real compositor checks before any queued show.
        // Normal and assertion-failure paths explicitly stop and await instead.
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    fn rect(width: f64) -> BrowserRect {
        BrowserRect {
            x: 24.0,
            y: 96.0,
            width,
            height: 300.0,
        }
    }

    #[tokio::test]
    async fn blocked_observer_does_not_stall_trusted_viewport_pulses() {
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        let mut heartbeat = ViewportHeartbeat::start_with(move |_| {
            observed.fetch_add(1, Ordering::AcqRel);
            std::future::ready(Ok(()))
        });
        heartbeat.resume("tab", rect(400.0)).await.unwrap();
        // Longer than the unchanged native 1.5s lease. The observer has not
        // returned, while its independent viewport owner must keep renewing.
        tokio::time::sleep(Duration::from_millis(1700)).await;
        assert!(count.load(Ordering::Acquire) >= 8);
        tokio::time::timeout(Duration::from_secs(2), heartbeat.stop())
            .await
            .unwrap()
            .unwrap();
        let stopped = count.load(Ordering::Acquire);
        tokio::time::sleep(PERIOD * 2).await;
        assert_eq!(count.load(Ordering::Acquire), stopped);
    }

    #[tokio::test]
    async fn pause_and_stop_wait_for_native_ack_and_prevent_any_late_show() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let count = Arc::new(AtomicUsize::new(0));
        let did_enter = entered.clone();
        let did_release = release.clone();
        let observed = count.clone();
        let heartbeat = Arc::new(ViewportHeartbeat::start_with(move |_| {
            let entered = did_enter.clone();
            let release = did_release.clone();
            let count = observed.clone();
            async move {
                count.fetch_add(1, Ordering::AcqRel);
                entered.notify_one();
                release.notified().await;
                Ok(())
            }
        }));
        heartbeat.resume("tab", rect(400.0)).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
        let pausing = heartbeat.clone();
        let pause = tokio::spawn(async move { pausing.pause().await });
        tokio::time::sleep(PERIOD).await;
        assert!(
            !pause.is_finished(),
            "pause returned before native show acknowledged"
        );
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), pause)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let paused = count.load(Ordering::Acquire);
        tokio::time::sleep(PERIOD * 2).await;
        assert_eq!(count.load(Ordering::Acquire), paused);
        let mut heartbeat = Arc::try_unwrap(heartbeat).ok().unwrap();
        heartbeat.resume("tab", rect(500.0)).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
        let stopping = tokio::spawn(async move {
            heartbeat.stop().await.unwrap();
            heartbeat
        });
        tokio::time::sleep(PERIOD).await;
        assert!(
            !stopping.is_finished(),
            "stop returned before native show acknowledged"
        );
        release.notify_one();
        let heartbeat = tokio::time::timeout(Duration::from_secs(2), stopping)
            .await
            .unwrap()
            .unwrap();
        assert!(heartbeat.resume("tab", rect(500.0)).await.is_err());
        let stopped = count.load(Ordering::Acquire);
        tokio::time::sleep(PERIOD * 2).await;
        assert_eq!(count.load(Ordering::Acquire), stopped);
    }
}
