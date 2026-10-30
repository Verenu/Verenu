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
}

pub(crate) struct Activity(Arc<ActivityGate>);

#[allow(dead_code)]
pub(crate) struct Pause(Arc<ActivityGate>);

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

    #[allow(dead_code)]
    pub(crate) async fn pause(self: &Arc<Self>) -> Result<Pause, &'static str> {
        {
            let mut state = self.state.lock().expect("activity gate lock");
            if state.paused {
                return Err("An update restart is already in progress.");
            }
            state.paused = true;
        }
        let pause = Pause(self.clone());
        loop {
            let notified = self.drained.notified();
            if self.state.lock().expect("activity gate lock").active == 0 {
                return Ok(pause);
            }
            notified.await;
        }
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
}
