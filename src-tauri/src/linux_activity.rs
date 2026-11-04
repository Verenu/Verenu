use std::sync::{Arc, Condvar, Mutex};
use tokio::sync::Notify;

#[derive(Default)]
struct State {
    paused: bool,
    active: usize,
}

#[derive(Default)]
pub(crate) struct ActivityGate {
    state: Mutex<State>,
    drained: Notify,
    resumed: Condvar,
    resumed_async: Notify,
}

pub(crate) struct Activity(Arc<ActivityGate>);

pub(crate) struct Pause(Arc<ActivityGate>);

impl Pause {
    pub(crate) fn belongs_to(&self, gate: &Arc<ActivityGate>) -> bool {
        Arc::ptr_eq(&self.0, gate)
    }

    pub(crate) async fn wait_drained(&self) {
        loop {
            let notified = self.0.drained.notified();
            if self.0.state.lock().expect("activity gate lock").active == 0 {
                return;
            }
            notified.await;
        }
    }
}

impl ActivityGate {
    pub(crate) fn try_enter(self: &Arc<Self>) -> Option<Activity> {
        let mut state = self.state.lock().expect("activity gate lock");
        if state.paused {
            return None;
        }
        state.active += 1;
        Some(Activity(self.clone()))
    }

    pub(crate) fn enter_blocking(self: &Arc<Self>) -> Activity {
        let state = self.state.lock().expect("activity gate lock");
        let mut state = self
            .resumed
            .wait_while(state, |state| state.paused)
            .expect("activity gate lock");
        state.active += 1;
        Activity(self.clone())
    }

    pub(crate) async fn enter(self: &Arc<Self>) -> Activity {
        loop {
            let notified = self.resumed_async.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(activity) = self.try_enter() {
                return activity;
            }
            notified.await;
        }
    }

    pub(crate) fn begin_pause(self: &Arc<Self>) -> Result<Pause, &'static str> {
        self.begin_pause_with(|| Ok(()))
    }

    pub(crate) fn begin_pause_with(
        self: &Arc<Self>,
        prepare: impl FnOnce() -> Result<(), &'static str>,
    ) -> Result<Pause, &'static str> {
        {
            let mut state = self.state.lock().expect("activity gate lock");
            if state.paused {
                return Err("An update restart is already in progress.");
            }
            prepare()?;
            state.paused = true;
        }
        Ok(Pause(self.clone()))
    }

    pub(crate) async fn pause(self: &Arc<Self>) -> Result<Pause, &'static str> {
        let pause = self.begin_pause()?;
        pause.wait_drained().await;
        Ok(pause)
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().expect("activity gate lock");
        state.active -= 1;
        if state.active == 0 {
            self.0.drained.notify_one();
        }
    }
}

impl Drop for Pause {
    fn drop(&mut self) {
        self.0.state.lock().expect("activity gate lock").paused = false;
        self.0.resumed.notify_all();
        self.0.resumed_async.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pause_drains_existing_work_and_rejects_new_work_until_resumed() {
        let gate = Arc::new(ActivityGate::default());
        let active = gate.try_enter().unwrap();
        let waiting_gate = gate.clone();
        let waiting = tokio::spawn(async move { waiting_gate.pause().await.unwrap() });
        tokio::task::yield_now().await;
        assert!(gate.try_enter().is_none());
        assert!(!waiting.is_finished());
        drop(active);
        let pause = waiting.await.unwrap();
        assert!(gate.try_enter().is_none());
        drop(pause);
        assert!(gate.try_enter().is_some());
    }

    #[tokio::test]
    async fn cancelling_a_pending_pause_restores_admission() {
        let gate = Arc::new(ActivityGate::default());
        let active = gate.try_enter().unwrap();
        let waiting_gate = gate.clone();
        let waiting = tokio::spawn(async move { waiting_gate.pause().await.unwrap() });
        tokio::task::yield_now().await;
        assert!(gate.try_enter().is_none());
        waiting.abort();
        let _ = waiting.await;
        assert!(gate.try_enter().is_some());
        drop(active);
    }

    #[tokio::test]
    async fn cancelling_a_drain_wait_does_not_release_its_pause() {
        let gate = Arc::new(ActivityGate::default());
        let active = gate.try_enter().unwrap();
        let pause = gate.begin_pause().unwrap();
        assert!(tokio::time::timeout(std::time::Duration::from_millis(50), pause.wait_drained()).await.is_err());
        assert!(gate.try_enter().is_none());
        drop(active);
        pause.wait_drained().await;
        assert!(gate.try_enter().is_none());
        drop(pause);
        assert!(gate.try_enter().is_some());
    }

    #[test]
    fn failed_pause_preparation_leaves_admission_open() {
        let gate = Arc::new(ActivityGate::default());
        assert!(gate.begin_pause_with(|| Err("preparation failed")).is_err());
        assert!(gate.try_enter().is_some());
    }

    #[tokio::test]
    async fn blocked_worker_resumes_after_pause() {
        let gate = Arc::new(ActivityGate::default());
        let pause = gate.pause().await.unwrap();
        let worker_gate = gate.clone();
        let (sent, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _activity = worker_gate.enter_blocking();
            sent.send(()).unwrap();
        });
        assert!(received
            .recv_timeout(std::time::Duration::from_millis(50))
            .is_err());
        drop(pause);
        received
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn asynchronous_waiters_resume_after_pause() {
        let gate = Arc::new(ActivityGate::default());
        let pause = gate.pause().await.unwrap();
        let worker_gate = gate.clone();
        let worker = tokio::spawn(async move { worker_gate.enter().await });
        tokio::task::yield_now().await;
        assert!(!worker.is_finished());
        drop(pause);
        let _activity = tokio::time::timeout(std::time::Duration::from_secs(2), worker)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn cancelled_caller_still_drains_its_blocking_work() {
        let gate = Arc::new(ActivityGate::default());
        let work_gate = gate.clone();
        let (started, started_rx) = tokio::sync::oneshot::channel();
        let (finish, finish_rx) = std::sync::mpsc::channel::<()>();
        let caller = tokio::spawn(async move {
            let activity = Arc::new(work_gate.enter().await);
            let blocking_activity = activity.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _activity = blocking_activity;
                let _ = started.send(());
                let _ = finish_rx.recv();
            })
            .await;
        });
        started_rx.await.unwrap();
        caller.abort();
        let _ = caller.await;
        let pause_gate = gate.clone();
        let pause = tokio::spawn(async move { pause_gate.pause().await.unwrap() });
        tokio::task::yield_now().await;
        assert!(!pause.is_finished());
        finish.send(()).unwrap();
        let _pause = tokio::time::timeout(std::time::Duration::from_secs(2), pause)
            .await
            .unwrap()
            .unwrap();
    }
}
