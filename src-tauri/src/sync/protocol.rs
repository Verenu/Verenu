use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio::time::{timeout, Duration};

pub const PROTOCOL_VERSION: u32 = 2;

pub const MAX_MESSAGE_BYTES: u32 = 64 * 1024 * 1024;

const FRAME_MEMORY_UNIT_BYTES: usize = 64 * 1024;
const FRAME_MEMORY_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const FRAME_MEMORY_PERMITS: u32 = (FRAME_MEMORY_BUDGET_BYTES / FRAME_MEMORY_UNIT_BYTES) as u32;

pub const MESSAGE_READ_TIMEOUT: Duration = Duration::from_secs(30);

fn frame_memory() -> Arc<Semaphore> {
    static MEMORY: OnceLock<Arc<Semaphore>> = OnceLock::new();
    MEMORY
        .get_or_init(|| Arc::new(Semaphore::new(FRAME_MEMORY_PERMITS as usize)))
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

pub async fn send_message<W: AsyncWrite + Unpin>(writer: &mut W, message: &Message) -> Result<()> {
    let body = serde_json::to_vec(message).context("encode message")?;
    let len = u32::try_from(body.len()).map_err(|_| anyhow!("message too large"))?;
    if len > MAX_MESSAGE_BYTES {
        return Err(anyhow!("message too large: {len} bytes"));
    }
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Message> {
    read_message_with_timeout(reader, MESSAGE_READ_TIMEOUT).await
}

pub async fn read_message_with_timeout<R: AsyncRead + Unpin>(
    reader: &mut R,
    deadline: Duration,
) -> Result<Message> {
    timeout(deadline, read_message_inner(reader))
        .await
        .map_err(|_| anyhow!("timed out while reading sync message"))?
}

async fn read_message_inner<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Message> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_be_bytes(len_bytes);
    if len > MAX_MESSAGE_BYTES {
        return Err(anyhow!("peer sent oversized message: {len} bytes"));
    }
    let permits = ((len as usize).saturating_add(FRAME_MEMORY_UNIT_BYTES - 1)
        / FRAME_MEMORY_UNIT_BYTES)
        .max(1) as u32;
    let _memory = frame_memory()
        .acquire_many_owned(permits)
        .await
        .map_err(|_| anyhow!("sync frame memory budget unavailable"))?;
    let mut body = vec![0u8; len as usize];
    reader.read_exact(&mut body).await?;
    let message: Message = serde_json::from_slice(&body).context("decode message from peer")?;
    Ok(message)
}
