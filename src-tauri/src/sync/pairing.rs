use anyhow::{anyhow, Result};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hkdf::Hkdf;
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};

use super::protocol::{read_message, send_message, Message};

const PAIRING_INFO: &[u8] = b"verenu-lan-pairing-v1";
const SPAKE_ID_A: &[u8] = b"verenu/initiator";
const SPAKE_ID_B: &[u8] = b"verenu/responder";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityExchange {
    pub device_uuid: String,
    pub device_name: String,
    pub cert_der: Vec<u8>,
    #[serde(default)]
    pub sync_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing_id: Option<String>,
}

pub fn generate_pairing_code() -> String {
    let value = rand::thread_rng().gen_range(0..1_000_000);
    format!("{value:06}")
}

fn derive_cipher(shared: &[u8; 32]) -> ChaCha20Poly1305 {
    let hk = Hkdf::<Sha256>::new(None, shared);
    let mut okm = [0u8; 32];
    hk.expand(PAIRING_INFO, &mut okm)
        .expect("32-byte OKM is valid for HKDF-SHA256");
    ChaCha20Poly1305::new((&okm).into())
}

fn encrypt_identity(
    cipher: &ChaCha20Poly1305,
    identity: &IdentityExchange,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let plaintext = serde_json::to_vec(identity)?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: &plaintext,
                aad: PAIRING_INFO,
            },
        )
        .map_err(|_| anyhow!("identity encryption failed"))?;
    Ok((ciphertext, nonce_bytes.to_vec()))
}

fn decrypt_identity(
    cipher: &ChaCha20Poly1305,
    ciphertext: &[u8],
    nonce: &[u8],
) -> Result<IdentityExchange> {
    let nonce: [u8; 12] = nonce.try_into().map_err(|_| anyhow!("bad nonce length"))?;
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: ciphertext,
                aad: PAIRING_INFO,
            },
        )
        .map_err(|_| anyhow!("that code didn't match"))?;
    Ok(serde_json::from_slice(&plaintext)?)
}

pub type PairingOutcome = IdentityExchange;

pub fn initiator_start(code: &str) -> (Spake2<Ed25519Group>, Vec<u8>) {
    Spake2::<Ed25519Group>::start_a(
        &Password::new(code.as_bytes()),
        &Identity::new(SPAKE_ID_A),
        &Identity::new(SPAKE_ID_B),
    )
}

pub fn responder_start(code: &str, initiator_msg: &[u8]) -> Result<(Vec<u8>, ChaCha20Poly1305)> {
    let (state, msg) = Spake2::<Ed25519Group>::start_b(
        &Password::new(code.as_bytes()),
        &Identity::new(SPAKE_ID_A),
        &Identity::new(SPAKE_ID_B),
    );
    let shared = state
        .finish(initiator_msg)
        .map_err(|_| anyhow!("pairing handshake failed"))?;
    let shared: [u8; 32] = shared.try_into().map_err(|_| anyhow!("bad key length"))?;
    Ok((msg, derive_cipher(&shared)))
}

pub fn initiator_cipher(
    state: Spake2<Ed25519Group>,
    responder_msg: &[u8],
) -> Result<ChaCha20Poly1305> {
    let shared = state
        .finish(responder_msg)
        .map_err(|_| anyhow!("pairing handshake failed"))?;
    let shared: [u8; 32] = shared.try_into().map_err(|_| anyhow!("bad key length"))?;
    Ok(derive_cipher(&shared))
}

pub async fn initiator_exchange<S>(
    stream: &mut S,
    cipher: &ChaCha20Poly1305,
    self_identity: &IdentityExchange,
    expected_peer_uuid: &str,
) -> Result<PairingOutcome>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (ciphertext, nonce) = encrypt_identity(cipher, self_identity)?;
    send_message(stream, &Message::PairVerify { ciphertext, nonce }).await?;

    let outcome = match read_message(stream).await? {
        Message::PairVerify { ciphertext, nonce } => decrypt_identity(cipher, &ciphertext, &nonce)?,
        Message::PairReject { reason } => return Err(anyhow!("pairing rejected: {reason}")),
        Message::Error { message } => return Err(anyhow!("pairing failed: {message}")),
        other => return Err(anyhow!("unexpected pairing message: {other:?}")),
    };
    if outcome.device_uuid != expected_peer_uuid {
        return Err(anyhow!("pairing peer identity changed mid-handshake"));
    }
    if outcome.pairing_id.is_some() && outcome.pairing_id != self_identity.pairing_id {
        return Err(anyhow!("pairing identifier did not match"));
    }
    match read_message(stream).await? {
        Message::PairComplete => {}
        Message::Error { message } => return Err(anyhow!("pairing failed: {message}")),
        other => return Err(anyhow!("unexpected pairing message: {other:?}")),
    }
    Ok(outcome)
}

pub async fn responder_exchange<S>(
    stream: &mut S,
    cipher: &ChaCha20Poly1305,
    responder_msg: Vec<u8>,
    self_identity: &IdentityExchange,
    expected_peer_uuid: &str,
) -> Result<PairingOutcome>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    send_message(
        stream,
        &Message::PairAccept {
            spake_msg: responder_msg,
        },
    )
    .await?;

    let (ciphertext, nonce) = match read_message(stream).await? {
        Message::PairVerify { ciphertext, nonce } => (ciphertext, nonce),
        Message::Error { message } => return Err(anyhow!("pairing failed: {message}")),
        other => return Err(anyhow!("unexpected pairing message: {other:?}")),
    };
    let peer = decrypt_identity(cipher, &ciphertext, &nonce)?;
    if peer.device_uuid != expected_peer_uuid {
        return Err(anyhow!("pairing peer identity changed mid-handshake"));
    }

    if peer.pairing_id.as_deref().is_some_and(|id| uuid::Uuid::parse_str(id).is_err()) {
        return Err(anyhow!("invalid pairing identifier"));
    }
    let mut response_identity = self_identity.clone();
    response_identity.pairing_id.clone_from(&peer.pairing_id);
    let (my_ciphertext, my_nonce) = encrypt_identity(cipher, &response_identity)?;
    send_message(
        stream,
        &Message::PairVerify {
            ciphertext: my_ciphertext,
            nonce: my_nonce,
        },
    )
    .await?;
    Ok(peer)
}
