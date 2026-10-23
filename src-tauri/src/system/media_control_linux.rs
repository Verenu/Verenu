use std::{sync::Mutex, time::Duration};
use zbus::{names::OwnedUniqueName, proxy::CacheProperties, Connection, Proxy};

const CALL_TIMEOUT: Duration = Duration::from_millis(500);
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Default)]
struct DesiredState {
    generation: u64,
    active: bool,
}

struct Controller {
    desired: Mutex<DesiredState>,
    // Serialize D-Bus work so cancellation cannot lose an in-flight pause.
    paused: tokio::sync::Mutex<Vec<OwnedUniqueName>>,
}

impl Controller {
    const fn new() -> Self {
        Self {
            desired: Mutex::new(DesiredState {
                generation: 0,
                active: false,
            }),
            paused: tokio::sync::Mutex::const_new(Vec::new()),
        }
    }

    fn desired(&self) -> std::sync::MutexGuard<'_, DesiredState> {
        self.desired.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn set_active(&self, active: bool, generation: Option<u64>) -> bool {
        let mut state = self.desired();
        if generation.is_some_and(|generation| !state.active || state.generation != generation) {
            return false;
        }
        state.generation = state.generation.wrapping_add(1);
        state.active = active;
        true
    }

    async fn reconcile(&self) -> zbus::Result<()> {
        let mut paused = self.paused.lock().await;
        let connection = tokio::time::timeout(
            CALL_TIMEOUT,
            zbus::connection::Builder::session()?
                .method_timeout(CALL_TIMEOUT)
                .build(),
        )
        .await
        .map_err(|_| zbus::Error::Failure("Media bus connection timed out".into()))??;

        if self.desired().active {
            let bus = zbus::fdo::DBusProxy::new(&connection).await?;
            for name in bus.list_names().await? {
                if !self.desired().active {
                    break;
                }
                if !name.as_str().starts_with("org.mpris.MediaPlayer2.") {
                    continue;
                }
                // Bind to this process, never a replacement that acquires its name.
                let Ok(owner) = bus.get_name_owner(name.as_ref()).await else {
                    continue;
                };
                match pause_player(&connection, &owner).await {
                    Ok(true) if !paused.contains(&owner) => paused.push(owner),
                    Ok(_) => {}
                    Err(err) => log::debug!("media pause: could not pause Linux player: {err}"),
                }
            }
        }

        if !self.desired().active {
            let mut retry = Vec::new();
            for owner in paused.drain(..) {
                if self.desired().active {
                    retry.push(owner);
                    continue;
                }
                if let Err(err) = resume_player(&connection, &owner).await {
                    log::debug!("media pause: could not resume Linux player: {err}");
                    // Retain ownership for a later restoration attempt.
                    retry.push(owner);
                }
            }
            *paused = retry;
        }
        Ok(())
    }
}

async fn player<'a>(
    connection: &'a Connection,
    owner: &'a OwnedUniqueName,
) -> zbus::Result<Proxy<'a>> {
    zbus::proxy::Builder::new(connection)
        .destination(owner.as_str())?
        .path(PLAYER_PATH)?
        .interface(PLAYER_INTERFACE)?
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

async fn pause_player(connection: &Connection, owner: &OwnedUniqueName) -> zbus::Result<bool> {
    let player = player(connection, owner).await?;
    if player.get_property::<String>("PlaybackStatus").await? != "Playing"
        || !player.get_property::<bool>("CanControl").await?
        || !player.get_property::<bool>("CanPause").await?
    {
        return Ok(false);
    }
    player.call::<_, _, ()>("Pause", &()).await?;
    Ok(true)
}

async fn resume_player(connection: &Connection, owner: &OwnedUniqueName) -> zbus::Result<()> {
    let bus = zbus::fdo::DBusProxy::new(connection).await?;
    if !bus.name_has_owner(owner.as_ref().into()).await? {
        return Ok(());
    }
    let player = player(connection, owner).await?;
    if player.get_property::<String>("PlaybackStatus").await? == "Paused"
        && player.get_property::<bool>("CanControl").await?
        && player.get_property::<bool>("CanPlay").await?
    {
        player.call::<_, _, ()>("Play", &()).await?;
    }
    Ok(())
}

static CONTROLLER: Controller = Controller::new();

fn reconcile() {
    tauri::async_runtime::spawn(async {
        if let Err(err) = CONTROLLER.reconcile().await {
            log::warn!("media pause: Linux media controls unavailable: {err}");
        }
    });
}

pub fn begin() {
    CONTROLLER.set_active(true, None);
    reconcile();
}

pub fn end() {
    CONTROLLER.set_active(false, None);
    reconcile();
}

pub fn active_generation() -> Option<u64> {
    let state = CONTROLLER.desired();
    state.active.then_some(state.generation)
}

pub fn end_if_current(generation: u64) {
    if CONTROLLER.set_active(false, Some(generation)) {
        reconcile();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct TestPlayer {
        status: Mutex<String>,
        can_pause: bool,
        reject_pause: bool,
        pauses: AtomicUsize,
        plays: AtomicUsize,
        pause_started: tokio::sync::Notify,
        release_pause: Option<Arc<tokio::sync::Notify>>,
    }

    struct Service(Arc<TestPlayer>);

    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl Service {
        async fn pause(&self) -> zbus::fdo::Result<()> {
            self.0.pauses.fetch_add(1, Ordering::SeqCst);
            self.0.pause_started.notify_one();
            if let Some(release) = &self.0.release_pause {
                release.notified().await;
            }
            if self.0.reject_pause {
                return Err(zbus::fdo::Error::Failed("Synthetic rejection".into()));
            }
            *self.0.status.lock().unwrap() = "Paused".into();
            Ok(())
        }

        fn play(&self) {
            self.0.plays.fetch_add(1, Ordering::SeqCst);
            *self.0.status.lock().unwrap() = "Playing".into();
        }

        #[zbus(property)]
        fn playback_status(&self) -> String {
            self.0.status.lock().unwrap().clone()
        }

        #[zbus(property)]
        fn can_pause(&self) -> bool {
            self.0.can_pause
        }

        #[zbus(property)]
        fn can_play(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_control(&self) -> bool {
            true
        }
    }

    async fn serve(
        name: &str,
        status: &str,
        can_pause: bool,
        reject_pause: bool,
        release_pause: Option<Arc<tokio::sync::Notify>>,
    ) -> (Connection, Arc<TestPlayer>) {
        let state = Arc::new(TestPlayer {
            status: Mutex::new(status.into()),
            can_pause,
            reject_pause,
            pauses: AtomicUsize::new(0),
            plays: AtomicUsize::new(0),
            pause_started: tokio::sync::Notify::new(),
            release_pause,
        });
        let connection = zbus::connection::Builder::session()
            .unwrap()
            .name(name)
            .unwrap()
            .serve_at(PLAYER_PATH, Service(state.clone()))
            .unwrap()
            .build()
            .await
            .unwrap();
        (connection, state)
    }

    #[test]
    fn stale_guard_cannot_release_linux_pause() {
        let controller = Controller::new();
        controller.set_active(true, None);
        let old = controller.desired().generation;
        controller.set_active(true, None);
        assert!(!controller.set_active(false, Some(old)));
        assert!(controller.desired().active);
        let current = controller.desired().generation;
        assert!(controller.set_active(false, Some(current)));
        assert!(!controller.desired().active);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires an isolated D-Bus session via dbus-run-session"]
    async fn linux_media_pause_dbus_lifecycle() {
        let controller = Arc::new(Controller::new());
        let (playing_bus, playing) = serve(
            "org.mpris.MediaPlayer2.verenu_playing",
            "Playing",
            true,
            false,
            None,
        )
        .await;
        let (_idle_bus, idle) = serve(
            "org.mpris.MediaPlayer2.verenu_idle",
            "Paused",
            true,
            false,
            None,
        )
        .await;
        let (_unsupported_bus, unsupported) = serve(
            "org.mpris.MediaPlayer2.verenu_unsupported",
            "Playing",
            false,
            false,
            None,
        )
        .await;
        let (_rejected_bus, rejected) = serve(
            "org.mpris.MediaPlayer2.verenu_rejected",
            "Playing",
            true,
            true,
            None,
        )
        .await;
        controller.set_active(true, None);
        controller.reconcile().await.unwrap();
        assert_eq!(*playing.status.lock().unwrap(), "Paused");
        assert_eq!(idle.pauses.load(Ordering::SeqCst), 0);
        assert_eq!(unsupported.pauses.load(Ordering::SeqCst), 0);
        assert_eq!(rejected.pauses.load(Ordering::SeqCst), 1);
        controller.set_active(false, None);
        controller.reconcile().await.unwrap();
        assert_eq!(*playing.status.lock().unwrap(), "Playing");
        assert_eq!(playing.plays.load(Ordering::SeqCst), 1);
        assert_eq!(idle.plays.load(Ordering::SeqCst), 0);
        assert_eq!(rejected.plays.load(Ordering::SeqCst), 0);

        // A new recording inherits the paused players without briefly resuming.
        controller.set_active(true, None);
        controller.reconcile().await.unwrap();
        controller.set_active(true, None);
        controller.reconcile().await.unwrap();
        assert_eq!(playing.plays.load(Ordering::SeqCst), 1);
        *playing.status.lock().unwrap() = "Stopped".into();
        controller.set_active(false, None);
        controller.reconcile().await.unwrap();
        assert_eq!(playing.plays.load(Ordering::SeqCst), 1);

        // A replacement process with the same MPRIS name is not ours to resume.
        *playing.status.lock().unwrap() = "Playing".into();
        controller.set_active(true, None);
        controller.reconcile().await.unwrap();
        playing_bus.close().await.unwrap();
        let (_replacement_bus, replacement) = serve(
            "org.mpris.MediaPlayer2.verenu_playing",
            "Paused",
            true,
            false,
            None,
        )
        .await;
        controller.set_active(false, None);
        controller.reconcile().await.unwrap();
        assert_eq!(replacement.plays.load(Ordering::SeqCst), 0);
        assert!(controller.paused.lock().await.is_empty());

        // Cancellation while Pause is in flight must still restore that player.
        let release = Arc::new(tokio::sync::Notify::new());
        let (_slow_bus, slow) = serve(
            "org.mpris.MediaPlayer2.verenu_slow",
            "Playing",
            true,
            false,
            Some(release.clone()),
        )
        .await;
        controller.set_active(true, None);
        let worker = {
            let controller = controller.clone();
            tokio::spawn(async move { controller.reconcile().await })
        };
        tokio::time::timeout(Duration::from_secs(2), slow.pause_started.notified())
            .await
            .unwrap();
        controller.set_active(false, None);
        release.notify_one();
        worker.await.unwrap().unwrap();
        assert_eq!(*slow.status.lock().unwrap(), "Playing");
        assert_eq!(slow.plays.load(Ordering::SeqCst), 1);
        assert!(controller.paused.lock().await.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires dbus-run-session, mpv, ffmpeg, and /usr/lib/mpv-mpris/mpris.so"]
    async fn linux_media_pause_real_mpv() {
        use std::process::{Child, Command, Stdio};
        struct Playback {
            child: Child,
            audio: std::path::PathBuf,
        }
        impl Drop for Playback {
            fn drop(&mut self) {
                let _ = self.child.kill();
                let _ = self.child.wait();
                let _ = std::fs::remove_file(&self.audio);
            }
        }
        let audio = std::env::temp_dir().join(format!("verenu-media-{}.wav", uuid::Uuid::new_v4()));
        assert!(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=16000:cl=mono",
                "-t",
                "30"
            ])
            .arg(&audio)
            .status()
            .unwrap()
            .success());
        let _playback = Playback {
            child: Command::new("mpv")
                .args([
                    "--no-config",
                    "--no-terminal",
                    "--no-video",
                    "--ao=null",
                    "--loop-file=inf",
                    "--script=/usr/lib/mpv-mpris/mpris.so",
                ])
                .arg(&audio)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
            audio,
        };
        let connection = Connection::session().await.unwrap();
        let bus = zbus::fdo::DBusProxy::new(&connection).await.unwrap();
        let owner = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                for name in bus.list_names().await.unwrap() {
                    if name.as_str().starts_with("org.mpris.MediaPlayer2.mpv") {
                        let owner = bus.get_name_owner(name.as_ref()).await.unwrap();
                        if player(&connection, &owner)
                            .await
                            .unwrap()
                            .get_property::<String>("PlaybackStatus")
                            .await
                            .unwrap()
                            == "Playing"
                        {
                            return owner;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let player = player(&connection, &owner).await.unwrap();
        let controller = Controller::new();
        controller.set_active(true, None);
        controller.reconcile().await.unwrap();
        assert_eq!(
            player
                .get_property::<String>("PlaybackStatus")
                .await
                .unwrap(),
            "Paused"
        );
        let paused_position = player.get_property::<i64>("Position").await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            player.get_property::<i64>("Position").await.unwrap(),
            paused_position
        );
        controller.set_active(false, None);
        controller.reconcile().await.unwrap();
        assert_eq!(
            player
                .get_property::<String>("PlaybackStatus")
                .await
                .unwrap(),
            "Playing"
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            while player.get_property::<i64>("Position").await.unwrap() <= paused_position {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
}
