use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::sync::{Arc, OnceLock};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio::time::{timeout, Duration};

pub const PROTOCOL_VERSION: u32 = 3;

pub const MAX_MESSAGE_BYTES: u32 = 64 * 1024 * 1024;
// Negotiated frames set the length's high bit and prefix gzip data with the
// uncompressed JSON length. Older peers only receive the original framing.
const COMPRESSED_FRAME: u32 = 1 << 31;
const COMPRESSION_MIN_BYTES: usize = 1024;

const FRAME_MEMORY_UNIT_BYTES: usize = 64 * 1024;
const FRAME_MEMORY_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const FRAME_MEMORY_PERMITS: u32 = (FRAME_MEMORY_BUDGET_BYTES / FRAME_MEMORY_UNIT_BYTES) as u32;
const OUTGOING_MEMORY_PERMITS: u32 = FRAME_MEMORY_PERMITS + 1;
const MUTE_CONTROL_MEMORY_PERMITS: usize = 64;
const MAX_MUTE_DEVICE_UUID_BYTES: usize = 128;
const MAX_MUTE_CONTROL_BYTES: usize = 1024;

pub const MESSAGE_READ_TIMEOUT: Duration = Duration::from_secs(30);

fn frame_memory() -> Arc<Semaphore> {
    static MEMORY: OnceLock<Arc<Semaphore>> = OnceLock::new();
    MEMORY
        .get_or_init(|| Arc::new(Semaphore::new(FRAME_MEMORY_PERMITS as usize)))
        .clone()
}

fn incoming_small_frame_memory() -> Arc<Semaphore> {
    // Frame type is unknown until decoding, so reserve space for any small
    // uncompressed frame independently of the bulk pool.
    static MEMORY: OnceLock<Arc<Semaphore>> = OnceLock::new();
    MEMORY
        .get_or_init(|| Arc::new(Semaphore::new(MUTE_CONTROL_MEMORY_PERMITS)))
        .clone()
}

fn outgoing_frame_memory() -> Arc<Semaphore> {
    // Separate inbound and outbound pools keep full-duplex peers from holding
    // all permits while each waits for the other to read a frame.
    static MEMORY: OnceLock<Arc<Semaphore>> = OnceLock::new();
    MEMORY
        .get_or_init(|| Arc::new(Semaphore::new(OUTGOING_MEMORY_PERMITS as usize)))
        .clone()
}

fn outgoing_mute_control_memory() -> Arc<Semaphore> {
    static MEMORY: OnceLock<Arc<Semaphore>> = OnceLock::new();
    MEMORY
        .get_or_init(|| Arc::new(Semaphore::new(MUTE_CONTROL_MEMORY_PERMITS)))
        .clone()
}

pub const OPS_PER_BATCH: usize = 400;

pub const SNAPSHOT_ROW_CHUNK: i64 = 400;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub device_uuid: String,
    pub device_name: String,
    pub protocol: u32,
    pub app_version: String,
    #[serde(default)]
    pub activity_receipts: bool,
    #[serde(default)]
    pub cleanup_cache: bool,
    #[serde(default)]
    pub vocabulary_usage: bool,
    #[serde(default, rename = "z", skip_serializing_if = "is_false")]
    pub compressed_frames: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

struct LimitedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

struct CountingWriter {
    len: usize,
    limit: usize,
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.len) {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        self.len += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        let required = self.bytes.len() + bytes.len();
        if required > self.bytes.capacity() {
            let target = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(required)
                .min(self.limit);
            self.bytes
                .try_reserve_exact(target - self.bytes.len())
                .map_err(|_| std::io::ErrorKind::OutOfMemory)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsExchange {
    pub self_stats: DeviceStatsDto,
    pub remote_stats: Vec<DeviceStatsDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStatsDto {
    pub device_id: String,
    pub total_words: i64,
    pub dictionary_fixes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictionaryCorrectionRow {
    pub context_uuid: String,
    pub dictionary_uuid: String,
    #[serde(default)]
    pub dictionary_term: String,
    pub mistake: String,
    #[serde(default)]
    pub auto_learned: bool,
    #[serde(default)]
    pub correction_count: i64,
    #[serde(default = "default_correction_confidence_tier")]
    pub confidence_tier: String,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    pub created_at: String,
}

fn default_correction_confidence_tier() -> String {
    "low".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingRecord {
    pub key: String,
    pub value: serde_json::Value,
    pub ts_ms: i64,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncOp {
    pub table: String,
    pub row_uuid: String,
    pub op: String,
    pub ts_ms: i64,
    pub origin: String,
    pub origin_seq: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl SyncOp {
    pub fn table_name(&self) -> &str {
        &self.table
    }

    pub fn op_name(&self) -> &str {
        &self.op
    }

    pub fn is_delete(&self) -> bool {
        self.op == "delete"
    }

    pub fn stamp(&self) -> (i64, &str, i64) {
        (self.ts_ms, self.origin.as_str(), self.origin_seq)
    }

    pub fn newer_than(&self, other: &(i64, String, i64)) -> bool {
        self.stamp() > (other.0, other.1.as_str(), other.2)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    pub since_seq: i64,
    pub snapshot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpsBatch {
    pub ops: Vec<SyncOp>,
    pub cursor: i64,
    pub done: bool,
    pub snapshot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    DictationMute {
        device_uuid: String,
    },
    DictationMuteAck {
        enabled: bool,
        #[serde(default)]
        busy: bool,
    },
    DictationMuteHeartbeat,
    DictationMuteEnd,
    Hello(Hello),
    HelloAck(Hello),
    Meta {
        stats: StatsExchange,
        settings: Vec<SettingRecord>,
    },
    PullRequest(PullRequest),
    Ops(OpsBatch),
    Ack {
        seq: i64,
    },
    SyncDone,
    Unpair {
        device_uuid: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing_id: Option<String>,
    },
    Error {
        message: String,
    },
    PairRequest {
        device_uuid: String,
        device_name: String,
        protocol: u32,
        spake_msg: Vec<u8>,
    },
    PairAccept {
        spake_msg: Vec<u8>,
    },
    PairVerify {
        ciphertext: Vec<u8>,
        nonce: Vec<u8>,
    },
    PairComplete,
    PairReject {
        reason: String,
    },
    PairBusy,
}

pub async fn send_message<W: AsyncWrite + Unpin>(writer: &mut W, message: Message) -> Result<()> {
    send_message_with_compression(writer, message, false).await
}

pub async fn send_message_with_compression<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: Message,
    compression: bool,
) -> Result<()> {
    let _consent = crate::legal_consent::network_permit().await?;
    crate::legal_consent::while_consented(timeout(
        Duration::from_secs(30),
        send_message_with_budgets(
            writer,
            message,
            compression,
            outgoing_frame_memory(),
            outgoing_mute_control_memory(),
        ),
    ))
    .await?
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timed out while writing sync message",
        )
    })?
}

async fn send_message_with_budgets<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: Message,
    compression: bool,
    bulk_budget: Arc<Semaphore>,
    control_budget: Arc<Semaphore>,
) -> Result<()> {
    let is_mute_control = match &message {
        Message::DictationMute { device_uuid } => device_uuid.len() <= MAX_MUTE_DEVICE_UUID_BYTES,
        Message::DictationMuteAck { .. }
        | Message::DictationMuteHeartbeat
        | Message::DictationMuteEnd => true,
        _ => false,
    };
    if !is_mute_control {
        return send_message_with_budget(writer, message, compression, bulk_budget).await;
    }
    // Keep bounded mute frames independent of bulk writes and blocking workers.
    let _memory = control_budget
        .acquire_owned()
        .await
        .map_err(|_| anyhow!("outgoing mute control memory budget unavailable"))?;
    let mut frame = LimitedBuffer {
        bytes: Vec::new(),
        limit: MAX_MUTE_CONTROL_BYTES + 4,
    };
    frame.write_all(&[0; 4])?;
    serde_json::to_writer(&mut frame, &message).context("encode mute control")?;
    let len = (frame.bytes.len() - 4) as u32;
    frame.bytes[..4].copy_from_slice(&len.to_be_bytes());
    writer.write_all(&frame.bytes).await?;
    writer.flush().await?;
    Ok(())
}

async fn send_message_with_budget<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: Message,
    compression: bool,
    budget: Arc<Semaphore>,
) -> Result<()> {
    let (message, len) = tokio::task::spawn_blocking(move || -> Result<_> {
        let mut counter = CountingWriter {
            len: 0,
            limit: MAX_MESSAGE_BYTES as usize,
        };
        serde_json::to_writer(&mut counter, &message).map_err(|error| {
            if error.io_error_kind() == Some(std::io::ErrorKind::WriteZero) {
                anyhow!("message too large")
            } else {
                anyhow!(error).context("encode message")
            }
        })?;
        Ok((message, counter.len as u32))
    })
    .await
    .context("message sizing task failed")??;
    let compression_limit = FRAME_MEMORY_BUDGET_BYTES
        .checked_sub(len as usize + 4)
        .map(|budget| budget.min((len as usize).saturating_sub(5)))
        .unwrap_or(0);
    let try_compression =
        compression && len as usize >= COMPRESSION_MIN_BYTES && compression_limit > 0;
    let reserved = len as usize
        + 12
        + if try_compression {
            compression_limit
        } else {
            0
        };
    let permits = reserved.div_ceil(FRAME_MEMORY_UNIT_BYTES).max(1) as u32;
    let memory = budget
        .acquire_many_owned(permits)
        .await
        .map_err(|_| anyhow!("outgoing sync frame memory budget unavailable"))?;
    let (frame, memory) = tokio::task::spawn_blocking(move || -> Result<_> {
        let mut frame_buffer = LimitedBuffer {
            bytes: Vec::new(),
            limit: len as usize + 4,
        };
        frame_buffer
            .bytes
            .try_reserve_exact(len as usize + 4)
            .map_err(|_| anyhow!("could not allocate outgoing sync frame"))?;
        frame_buffer.write_all(&[0; 4])?;
        serde_json::to_writer(&mut frame_buffer, &message).context("encode message")?;
        if frame_buffer.bytes.len() != len as usize + 4 {
            return Err(anyhow!("sync message changed while encoding"));
        }
        let mut frame = frame_buffer.bytes;
        if try_compression {
            let buffer = LimitedBuffer {
                bytes: Vec::new(),
                limit: compression_limit,
            };
            let mut encoder = flate2::write::GzEncoder::new(buffer, flate2::Compression::fast());
            let result = match encoder.write_all(&frame[4..]) {
                Ok(()) => encoder.finish(),
                Err(error) => Err(error),
            };
            let compressed = match result {
                Ok(buffer) => Some(buffer.bytes),
                Err(error) if error.kind() == std::io::ErrorKind::WriteZero => None,
                Err(error) => return Err(error.into()),
            };
            if let Some(compressed) = compressed {
                drop(frame);
                frame = Vec::with_capacity(compressed.len() + 8);
                let wire_len = (compressed.len() + 4) as u32;
                frame.extend_from_slice(&(wire_len | COMPRESSED_FRAME).to_be_bytes());
                frame.extend_from_slice(&len.to_be_bytes());
                frame.extend_from_slice(&compressed);
            } else {
                frame[..4].copy_from_slice(&len.to_be_bytes());
            }
        } else {
            frame[..4].copy_from_slice(&len.to_be_bytes());
        }
        Ok((frame, memory))
    })
    .await
    .context("message encoding task failed")??;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    drop(memory);
    Ok(())
}

pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Message> {
    read_message_with_timeout(reader, MESSAGE_READ_TIMEOUT).await
}

pub async fn read_message_with_timeout<R: AsyncRead + Unpin>(
    reader: &mut R,
    deadline: Duration,
) -> Result<Message> {
    tokio::select! {
        result = timeout(deadline, read_message_inner(reader)) => result
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out while reading sync message"))?,
        _ = async {
            loop {
                if crate::legal_consent::require_consent().is_err() { break; }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        } => Err(anyhow!("consent_required")),
    }
}

async fn read_message_inner<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Message> {
    read_message_with_budgets(reader, frame_memory(), incoming_small_frame_memory()).await
}

async fn read_message_with_budgets<R: AsyncRead + Unpin>(
    reader: &mut R,
    bulk_budget: Arc<Semaphore>,
    small_budget: Arc<Semaphore>,
) -> Result<Message> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let header = u32::from_be_bytes(len_bytes);
    let compressed = header & COMPRESSED_FRAME != 0;
    let len = header & !COMPRESSED_FRAME;
    if len > MAX_MESSAGE_BYTES {
        return Err(anyhow!("peer sent oversized message: {len} bytes"));
    }
    let expanded_len = if compressed {
        if len < 4 {
            return Err(anyhow!("invalid compressed frame"));
        }
        reader.read_exact(&mut len_bytes).await?;
        let expanded = u32::from_be_bytes(len_bytes);
        if expanded > MAX_MESSAGE_BYTES
            || (len as usize + expanded as usize) > FRAME_MEMORY_BUDGET_BYTES
        {
            return Err(anyhow!("peer sent oversized compressed frame"));
        }
        expanded as usize
    } else {
        0
    };
    let permits = ((len as usize + expanded_len).saturating_add(FRAME_MEMORY_UNIT_BYTES - 1)
        / FRAME_MEMORY_UNIT_BYTES)
        .max(1) as u32;
    let (budget, permits) = if !compressed && len as usize <= MAX_MUTE_CONTROL_BYTES {
        (small_budget, 1)
    } else {
        (bulk_budget, permits)
    };
    let memory = budget
        .acquire_many_owned(permits)
        .await
        .map_err(|_| anyhow!("sync frame memory budget unavailable"))?;
    let mut body = vec![0u8; len as usize - if compressed { 4 } else { 0 }];
    reader.read_exact(&mut body).await?;
    let message: Message = if compressed {
        tokio::task::spawn_blocking(move || -> Result<Message> {
            let _memory = memory;
            let mut decoded = vec![0; expanded_len];
            let mut decoder = flate2::bufread::GzDecoder::new(&body[..]);
            decoder
                .read_exact(&mut decoded)
                .map_err(|error| anyhow!("invalid compressed frame: {error}"))?;
            let mut extra = [0];
            if decoder
                .read(&mut extra)
                .map_err(|error| anyhow!("invalid compressed frame: {error}"))?
                != 0
            {
                return Err(anyhow!("compressed frame length mismatch"));
            }
            if !decoder.into_inner().is_empty() {
                return Err(anyhow!("trailing compressed frame data"));
            }
            serde_json::from_slice(&decoded).context("decode message from peer")
        })
        .await
        .context("decompression task failed")??
    } else {
        let _memory = memory;
        serde_json::from_slice(&body).context("decode message from peer")?
    };
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::{
        read_message_with_budgets, send_message_with_budget, send_message_with_budgets,
        LimitedBuffer, Message, Semaphore, Write, MAX_MESSAGE_BYTES,
    };
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn inbound_heartbeat_does_not_wait_for_a_blocked_bulk_frame() {
        let bulk = Arc::new(Semaphore::new(1));
        let small = Arc::new(Semaphore::new(1));
        let (mut sender, mut reader) = tokio::io::duplex(32);
        // Reserve the bulk pool for a compressed frame whose body never arrives.
        sender
            .write_all(&(super::COMPRESSED_FRAME | 8).to_be_bytes())
            .await
            .unwrap();
        sender.write_all(&32768u32.to_be_bytes()).await.unwrap();
        let first_bulk = bulk.clone();
        let first_small = small.clone();
        let first = tokio::spawn(async move {
            read_message_with_budgets(&mut reader, first_bulk, first_small).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while bulk.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!first.is_finished());

        let body = serde_json::to_vec(&Message::DictationMuteHeartbeat).unwrap();
        let mut wire = (body.len() as u32).to_be_bytes().to_vec();
        wire.extend_from_slice(&body);
        let received = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            read_message_with_budgets(&mut wire.as_slice(), bulk.clone(), small.clone()),
        )
        .await
        .expect("inbound heartbeat was blocked by bulk sync")
        .unwrap();
        assert!(matches!(received, Message::DictationMuteHeartbeat));
        assert_eq!(bulk.available_permits(), 0);
        assert_eq!(small.available_permits(), 1);
        first.abort();
        let _ = first.await;
        assert_eq!(bulk.available_permits(), 1);
    }

    #[tokio::test]
    async fn inbound_small_frames_share_a_bounded_budget() {
        let bulk = Arc::new(Semaphore::new(1));
        let small = Arc::new(Semaphore::new(1));
        let held = small.clone().acquire_owned().await.unwrap();
        let body = serde_json::to_vec(&Message::DictationMuteHeartbeat).unwrap();
        let mut wire = (body.len() as u32).to_be_bytes().to_vec();
        wire.extend_from_slice(&body);
        let mut reader = wire.as_slice();
        let read = read_message_with_budgets(&mut reader, bulk.clone(), small.clone());
        tokio::pin!(read);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(50), &mut read)
            .await
            .is_err());
        assert_eq!(bulk.available_permits(), 1);
        drop(held);
        assert!(matches!(
            read.await.unwrap(),
            Message::DictationMuteHeartbeat
        ));
        assert_eq!(small.available_permits(), 1);
    }

    #[test]
    fn compression_buffer_rejects_bytes_past_its_limit() {
        let mut buffer = LimitedBuffer {
            bytes: Vec::new(),
            limit: 3,
        };
        buffer.write_all(b"abc").unwrap();
        assert_eq!(
            buffer.write_all(b"d").unwrap_err().kind(),
            std::io::ErrorKind::WriteZero
        );
        assert_eq!(buffer.bytes, b"abc");
    }

    #[test]
    fn gzip_preserves_the_limit_error_for_unprofitable_frames() {
        let buffer = LimitedBuffer {
            bytes: Vec::new(),
            limit: 1,
        };
        let mut encoder = flate2::write::GzEncoder::new(buffer, flate2::Compression::fast());
        let result = match encoder.write_all(b"synthetic message") {
            Ok(()) => encoder.finish().map(|_| ()),
            Err(error) => Err(error),
        };
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::WriteZero);
    }

    #[tokio::test]
    async fn oversized_message_is_rejected_before_reserving_or_writing() {
        let message = Message::Error {
            message: "x".repeat(MAX_MESSAGE_BYTES as usize),
        };
        let budget = Arc::new(Semaphore::new(1));
        let (mut sender, mut receiver) = tokio::io::duplex(1);
        let error = send_message_with_budget(&mut sender, message, false, budget.clone())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("message too large"));
        assert_eq!(budget.available_permits(), 1);
        drop(sender);
        let mut bytes = Vec::new();
        receiver.read_to_end(&mut bytes).await.unwrap();
        assert!(bytes.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn large_encoding_does_not_block_send_timeout() {
        let message = Message::Error {
            message: "x".repeat(MAX_MESSAGE_BYTES as usize),
        };
        let mut sink = tokio::io::sink();
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(1),
            send_message_with_budget(&mut sink, message, false, Arc::new(Semaphore::new(1))),
        )
        .await;
        assert!(
            result.is_err(),
            "a large encode must not prevent its timeout from firing"
        );
    }

    #[tokio::test]
    async fn concurrent_sends_share_the_outgoing_budget() {
        let budget = Arc::new(Semaphore::new(1));
        let (mut writer, mut reader) = tokio::io::duplex(1);
        let first_budget = budget.clone();
        let first = tokio::spawn(async move {
            send_message_with_budget(&mut writer, Message::Ack { seq: 1 }, false, first_budget)
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while budget.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let second_budget = budget.clone();
        let mut second = tokio::spawn(async move {
            let mut sink = tokio::io::sink();
            send_message_with_budget(&mut sink, Message::Ack { seq: 2 }, false, second_budget).await
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut second)
                .await
                .is_err()
        );
        let drain = tokio::spawn(async move {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await.unwrap();
            bytes
        });
        tokio::time::timeout(std::time::Duration::from_secs(30), first)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(30), second)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!drain.await.unwrap().is_empty());
        assert_eq!(budget.available_permits(), 1);
    }

    #[tokio::test]
    async fn mute_heartbeat_does_not_wait_for_a_blocked_bulk_frame() {
        let bulk = Arc::new(Semaphore::new(1));
        let control = Arc::new(Semaphore::new(1));
        let (mut writer, mut reader) = tokio::io::duplex(1024);
        let first_bulk = bulk.clone();
        let first_control = control.clone();
        let first = tokio::spawn(async move {
            send_message_with_budgets(
                &mut writer,
                Message::Error {
                    message: "x".repeat(32 * 1024),
                },
                false,
                first_bulk,
                first_control,
            )
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while bulk.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut prefix = [0; 1];
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            reader.read_exact(&mut prefix),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            !first.is_finished(),
            "bulk frame must still be blocked on its writer"
        );
        let mut sink = tokio::io::sink();
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            send_message_with_budgets(
                &mut sink,
                Message::DictationMuteHeartbeat,
                false,
                bulk.clone(),
                control.clone(),
            ),
        )
        .await
        .expect("heartbeat was blocked by bulk sync")
        .unwrap();
        assert_eq!(bulk.available_permits(), 0);
        assert_eq!(control.available_permits(), 1);
        send_message_with_budgets(
            &mut sink,
            Message::DictationMute {
                device_uuid: "\0".repeat(super::MAX_MUTE_DEVICE_UUID_BYTES),
            },
            false,
            bulk.clone(),
            control.clone(),
        )
        .await
        .expect("bounded mute control should fit its reserved path");
        let oversized_control = send_message_with_budgets(
            &mut sink,
            Message::DictationMute {
                device_uuid: "x".repeat(super::MAX_MUTE_DEVICE_UUID_BYTES + 1),
            },
            false,
            bulk.clone(),
            control.clone(),
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), oversized_control)
                .await
                .is_err()
        );
        assert_eq!(control.available_permits(), 1);
        let drain = tokio::spawn(async move {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await.unwrap();
            bytes
        });
        tokio::time::timeout(std::time::Duration::from_secs(30), first)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!drain.await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn compressed_frame_round_trips_after_blocking_encode() {
        let text = "repeatable text ".repeat(1024);
        let (mut writer, mut reader) = tokio::io::duplex(65536);
        let message = Message::Error {
            message: text.clone(),
        };
        let (sent, received) = tokio::join!(
            super::send_message_with_compression(&mut writer, message, true),
            super::read_message(&mut reader)
        );
        sent.unwrap();
        match received.unwrap() {
            Message::Error { message } => assert_eq!(message, text),
            _ => panic!("unexpected message"),
        }
    }
}
