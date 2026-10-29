//! A session-owned PipeWire echo-cancel source for the default microphone.
//!
//! The module runs in a child `pw-cli` process rather than the user's daemon.
//! Closing the child's stdin removes its nodes if the application exits.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(0);

pub(super) struct EchoSource {
    child: Child,
    pub(super) name: String,
}

pub(super) struct EchoCapture {
    child: Child,
    reader: Option<std::thread::JoinHandle<()>>,
    stopping: Arc<AtomicBool>,
    _source: EchoSource,
}

impl EchoCapture {
    pub(super) fn start(
        source: EchoSource,
        mut samples: impl FnMut(&[f32]) + Send + 'static,
        failed: impl FnOnce() + Send + 'static,
    ) -> Result<Self> {
        let mut child = Command::new("pw-record")
            .args([
                "--target",
                &source.name,
                "--rate",
                "48000",
                "--channels",
                "1",
                "--format",
                "f32",
                "--latency",
                "20ms",
                "--raw",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("pw-record is unavailable")?;
        let mut stdout = child
            .stdout
            .take()
            .context("missing PipeWire capture pipe")?;
        let stopping = Arc::new(AtomicBool::new(false));
        let stopping_reader = Arc::clone(&stopping);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            let mut bytes = [0u8; 3840];
            let mut values = [0f32; 960];
            let mut ready_tx = Some(ready_tx);
            loop {
                if stdout.read_exact(&mut bytes).is_err() {
                    if !stopping_reader.load(Ordering::Acquire) {
                        failed();
                    }
                    break;
                }
                for (value, chunk) in values.iter_mut().zip(bytes.as_chunks::<4>().0) {
                    *value = f32::from_le_bytes(*chunk);
                }
                samples(&values);
                if let Some(tx) = ready_tx.take() {
                    let _ = tx.send(());
                }
            }
        });
        let capture = Self {
            child,
            reader: Some(reader),
            stopping,
            _source: source,
        };
        ready_rx
            .recv_timeout(Duration::from_secs(3))
            .context("PipeWire echo capture did not start")?;
        Ok(capture)
    }
}

impl Drop for EchoCapture {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn pipewire_dump() -> Result<Value> {
    let mut child = Command::new("pw-dump")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("pw-dump is unavailable")?;
    let mut stdout = child
        .stdout
        .take()
        .context("missing PipeWire inspection pipe")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("PipeWire inspection failed"))??;
    let status = status.context("PipeWire inspection timed out")?;
    if !status.success() {
        bail!("could not inspect PipeWire devices");
    }
    serde_json::from_slice(&bytes).context("invalid PipeWire device information")
}

fn default_source(dump: &Value) -> Option<&str> {
    let objects = dump.as_array()?;
    let name = objects.iter().find_map(|object| {
        object
            .get("metadata")?
            .as_array()?
            .iter()
            .find_map(|entry| {
                (entry.get("key")?.as_str()? == "default.audio.source")
                    .then(|| entry.get("value")?.get("name")?.as_str())
                    .flatten()
            })
    })?;
    objects
        .iter()
        .any(|object| {
            let props = object.get("info").and_then(|i| i.get("props"));
            props
                .and_then(|p| p.get("node.name"))
                .and_then(Value::as_str)
                == Some(name)
                && props
                    .and_then(|p| p.get("media.class"))
                    .and_then(Value::as_str)
                    == Some("Audio/Source")
        })
        .then_some(name)
}

fn module_command(source: &str, name: &str) -> Result<String> {
    let source = serde_json::to_string(source)?;
    let name = serde_json::to_string(name)?;
    Ok(format!(
        "load-module libpipewire-module-echo-cancel {{ monitor.mode = true \
         capture.props = {{ target.object = {source} }} \
         source.props = {{ node.name = {name} priority.session = -1000 }} \
         aec.args = {{ webrtc.noise_suppression = false webrtc.gain_control = false \
         webrtc.transient_suppression = false }} }}\n"
    ))
}

impl EchoSource {
    pub(super) fn start() -> Result<Self> {
        let dump = pipewire_dump()?;
        let microphone = default_source(&dump).context("no default PipeWire microphone")?;
        let name = format!(
            "verenu.aec.{}.{}",
            std::process::id(),
            NEXT_SOURCE.fetch_add(1, Ordering::Relaxed)
        );
        let mut child = Command::new("pw-cli")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("pw-cli is unavailable")?;
        let command = module_command(microphone, &name)?;
        if let Err(error) = child
            .stdin
            .as_mut()
            .context("missing PipeWire control pipe")?
            .write_all(command.as_bytes())
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        let mut source = Self { child, name };
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if source.child.try_wait()?.is_some() {
                bail!("PipeWire echo cancellation exited during startup");
            }
            if pipewire_dump()?.as_array().is_some_and(|objects| {
                objects.iter().any(|object| {
                    object
                        .get("info")
                        .and_then(|i| i.get("props"))
                        .and_then(|p| p.get("node.name"))
                        .and_then(Value::as_str)
                        == Some(&source.name)
                })
            }) {
                return Ok(source);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        bail!("PipeWire echo-cancel source did not start")
    }
}

impl Drop for EchoSource {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requires_a_real_default_source_before_starting_the_module() {
        let metadata =
            json!({"metadata": [{"key": "default.audio.source", "value": {"name": "mic"}}]});
        assert_eq!(default_source(&json!([metadata.clone()])), None);
        let node = json!({"info": {"props": {"node.name": "mic", "media.class": "Audio/Source"}}});
        let dump = json!([metadata, node]);
        assert_eq!(default_source(&dump), Some("mic"));
    }

    #[test]
    fn control_command_quotes_names_and_disables_extra_processing() {
        let command = module_command("mic\"name", "verenu.test").unwrap();
        assert!(command.contains("target.object = \"mic\\\"name\""));
        assert!(command.contains("monitor.mode = true"));
        assert!(command.contains("webrtc.noise_suppression = false"));
        assert!(command.contains("webrtc.gain_control = false"));
        assert!(command.contains("webrtc.transient_suppression = false"));
        assert!(command.ends_with('\n'));
    }

    #[test]
    #[ignore = "requires a disposable PipeWire server and microphone"]
    fn private_pipewire_capture_starts_and_removes_its_source() {
        let before = pipewire_dump().unwrap();
        let original_default = default_source(&before).unwrap().to_owned();
        let source = EchoSource::start().expect("create echo source");
        let name = source.name.clone();
        let during = pipewire_dump().unwrap();
        assert_eq!(default_source(&during), Some(original_default.as_str()));
        let (tx, rx) = mpsc::sync_channel(1);
        let mut tx = Some(tx);
        let capture = EchoCapture::start(
            source,
            move |buffer| {
                if let Some(tx) = tx.take() {
                    let _ = tx.send(buffer.len());
                }
            },
            || panic!("capture stopped unexpectedly"),
        )
        .expect("start echo capture");
        assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap() > 0);
        drop(capture);
        let dump = pipewire_dump().unwrap();
        assert!(!dump.as_array().unwrap().iter().any(|object| {
            object
                .get("info")
                .and_then(|i| i.get("props"))
                .and_then(|p| p.get("node.name"))
                .and_then(Value::as_str)
                == Some(name.as_str())
        }));
    }
}
