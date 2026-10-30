use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Request {
    pub(super) generation: u64,
    pub(super) active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum State {
    Starting,
    Active,
    Suspended,
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Acknowledgement {
    pub(super) generation: u64,
    pub(super) state: State,
}

#[derive(Clone)]
pub(super) struct Control {
    requests: watch::Sender<Request>,
    acknowledgements: watch::Receiver<Acknowledgement>,
    generation: Arc<Mutex<u64>>,
}

pub(super) struct Driver {
    pub(super) requests: watch::Receiver<Request>,
    acknowledgements: watch::Sender<Acknowledgement>,
}

pub(super) struct Pause {
    control: Control,
    generation: u64,
    resumed: bool,
}

impl Pause {
    pub(super) fn begin(control: &Control) -> Result<Self, String> {
        let mut counter = control
            .generation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let request = *control.requests.borrow();
        let acknowledgement = control.current();
        if !request.active
            || acknowledgement.generation != request.generation
            || acknowledgement.state != State::Active
        {
            return Err("Global shortcuts are not ready for restart".into());
        }
        *counter = counter
            .checked_add(1)
            .ok_or("Shortcut request counter exhausted")?;
        let generation = *counter;
        control.requests.send_replace(Request {
            generation,
            active: false,
        });
        Ok(Self {
            control: control.clone(),
            generation,
            resumed: false,
        })
    }

    pub(super) async fn wait(&self) -> Result<(), String> {
        match self.control.wait_for(self.generation).await? {
            State::Suspended => Ok(()),
            state => Err(format!("Could not suspend global shortcuts: {state:?}")),
        }
    }

    pub(super) async fn resume(mut self) -> Result<(), String> {
        let generation = self.control.request(true)?;
        match self.control.wait_for(generation).await? {
            State::Active => {
                self.resumed = true;
                Ok(())
            }
            state => Err(format!("Could not restore global shortcuts: {state:?}")),
        }
    }
}

impl Drop for Pause {
    fn drop(&mut self) {
        if !self.resumed {
            let _ = self.control.request(true);
        }
    }
}

impl Control {
    pub(super) fn new() -> (Self, Driver) {
        let initial = Request {
            generation: 0,
            active: true,
        };
        let (requests, request_rx) = watch::channel(initial);
        let (ack_tx, acknowledgements) = watch::channel(Acknowledgement {
            generation: 0,
            state: State::Starting,
        });
        (
            Self {
                requests,
                acknowledgements,
                generation: Arc::new(Mutex::new(0)),
            },
            Driver {
                requests: request_rx,
                acknowledgements: ack_tx,
            },
        )
    }

    pub(super) fn request(&self, active: bool) -> Result<u64, String> {
        let mut generation = self
            .generation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *generation = generation
            .checked_add(1)
            .ok_or("Shortcut request counter exhausted")?;
        self.requests.send_replace(Request {
            generation: *generation,
            active,
        });
        Ok(*generation)
    }

    pub(super) fn current(&self) -> Acknowledgement {
        self.acknowledgements.borrow().clone()
    }

    pub(super) async fn wait_for(&self, generation: u64) -> Result<State, String> {
        let mut acknowledgements = self.acknowledgements.clone();
        loop {
            let acknowledgement = acknowledgements.borrow_and_update().clone();
            if acknowledgement.generation == generation && acknowledgement.state != State::Starting
            {
                return Ok(acknowledgement.state);
            }
            if acknowledgement.generation > generation {
                return Err("Shortcut request was superseded".into());
            }
            acknowledgements
                .changed()
                .await
                .map_err(|_| "Shortcut service stopped".to_string())?;
        }
    }
}

impl Driver {
    pub(super) fn current(&self) -> Request {
        *self.requests.borrow()
    }

    pub(super) fn acknowledge(&self, request: Request, state: State) {
        self.acknowledgements.send_replace(Acknowledgement {
            generation: request.generation,
            state,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acknowledgement_must_match_the_current_request() {
        let (control, driver) = Control::new();
        let stop = control.request(false).unwrap();
        let start = control.request(true).unwrap();
        driver.acknowledge(
            Request {
                generation: stop,
                active: false,
            },
            State::Suspended,
        );
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(20),
            control.wait_for(start)
        )
        .await
        .is_err());
        driver.acknowledge(
            Request {
                generation: start,
                active: true,
            },
            State::Active,
        );
        assert_eq!(control.wait_for(start).await.unwrap(), State::Active);
        assert!(control.wait_for(stop).await.is_err());
    }

    #[tokio::test]
    async fn stopping_the_driver_wakes_waiters_with_an_error() {
        let (control, driver) = Control::new();
        let request = control.request(false).unwrap();
        drop(driver);
        assert!(control.wait_for(request).await.is_err());
    }

    #[tokio::test]
    async fn canceled_pause_requests_restoration_even_before_stop_is_acknowledged() {
        let (control, mut driver) = Control::new();
        driver.acknowledge(driver.current(), State::Active);
        let pause = Pause::begin(&control).unwrap();
        assert!(!driver.current().active);
        let stop = driver.current().generation;
        drop(pause);
        let resume = driver.current();
        assert!(resume.active);
        assert!(resume.generation > stop);
        driver.requests.borrow_and_update();
        driver.acknowledge(resume, State::Active);
        assert_eq!(
            control.wait_for(resume.generation).await.unwrap(),
            State::Active
        );
    }

    #[tokio::test]
    async fn pause_refuses_to_start_when_shortcuts_are_unavailable() {
        let (control, driver) = Control::new();
        assert!(Pause::begin(&control).is_err());
        driver.acknowledge(
            driver.current(),
            State::Unavailable("portal missing".into()),
        );
        assert!(Pause::begin(&control).is_err());
    }

    #[tokio::test]
    async fn resume_waits_for_reacquisition_and_cancellation_retries_it() {
        let (control, driver) = Control::new();
        driver.acknowledge(driver.current(), State::Active);
        let pause = Pause::begin(&control).unwrap();
        let stop = driver.current();
        driver.acknowledge(stop, State::Suspended);
        pause.wait().await.unwrap();
        let mut resume = Box::pin(pause.resume());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut resume)
                .await
                .is_err()
        );
        let first = driver.current();
        assert!(first.active);
        drop(resume);
        let retry = driver.current();
        assert!(retry.active && retry.generation > first.generation);
        driver.acknowledge(retry, State::Active);
        assert_eq!(
            control.wait_for(retry.generation).await.unwrap(),
            State::Active
        );
    }

    #[cfg(feature = "native-testing")]
    #[tokio::test]
    async fn suspend_acknowledgement_follows_dbus_release_and_resume_reacquires() {
        let name = format!(
            "com.verenu.ShortcutControl.x{}",
            uuid::Uuid::new_v4().simple()
        );
        let observer = zbus::Connection::session().await.unwrap();
        let (control, mut driver) = Control::new();
        let task_name = name.clone();
        let task = tokio::spawn(async move {
            let mut connection = Some(
                zbus::connection::Builder::session()
                    .unwrap()
                    .name(task_name.as_str())
                    .unwrap()
                    .build()
                    .await
                    .unwrap(),
            );
            driver.acknowledge(driver.current(), State::Active);
            while driver.requests.changed().await.is_ok() {
                let request = driver.current();
                if request.active {
                    if connection.is_none() {
                        connection = Some(
                            zbus::connection::Builder::session()
                                .unwrap()
                                .name(task_name.as_str())
                                .unwrap()
                                .build()
                                .await
                                .unwrap(),
                        );
                    }
                    driver.acknowledge(request, State::Active);
                } else {
                    if let Some(connection) = connection.take() {
                        connection.close().await.unwrap();
                    }
                    let bus = zbus::Connection::session().await.unwrap();
                    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
                    loop {
                        let owned: bool = bus
                            .call_method(
                                Some("org.freedesktop.DBus"),
                                "/org/freedesktop/DBus",
                                Some("org.freedesktop.DBus"),
                                "NameHasOwner",
                                &(task_name.as_str(),),
                            )
                            .await
                            .unwrap()
                            .body()
                            .deserialize()
                            .unwrap();
                        if !owned {
                            break;
                        }
                        assert!(
                            tokio::time::Instant::now() < deadline,
                            "shortcut name remained owned"
                        );
                        tokio::task::yield_now().await;
                    }
                    driver.acknowledge(request, State::Suspended);
                }
            }
        });
        let deadline = std::time::Duration::from_secs(10);
        assert_eq!(
            tokio::time::timeout(deadline, control.wait_for(0))
                .await
                .unwrap()
                .unwrap(),
            State::Active
        );
        let stop = control.request(false).unwrap();
        assert_eq!(
            tokio::time::timeout(deadline, control.wait_for(stop))
                .await
                .unwrap()
                .unwrap(),
            State::Suspended
        );
        let owned: bool = observer
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "NameHasOwner",
                &(name.as_str(),),
            )
            .await
            .unwrap()
            .body()
            .deserialize()
            .unwrap();
        assert!(!owned);
        let start = control.request(true).unwrap();
        assert_eq!(
            tokio::time::timeout(deadline, control.wait_for(start))
                .await
                .unwrap()
                .unwrap(),
            State::Active
        );
        let owned: bool = observer
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "NameHasOwner",
                &(name.as_str(),),
            )
            .await
            .unwrap()
            .body()
            .deserialize()
            .unwrap();
        assert!(owned);
        task.abort();
        let _ = task.await;
    }
}
