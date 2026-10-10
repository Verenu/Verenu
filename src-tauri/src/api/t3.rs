//! Existing T3 OAuth pairing and Effect JSON WebSocket RPC, bounded and redacted.
use crate::system::t3_skills::{self, Catalog, Skill};
use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, Url};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, time::Duration};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};

const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);

pub struct PairingTarget {
    pub base_url: Url,
    pub credential: String,
}

pub fn parse_pairing_link(input: &str) -> Result<PairingTarget, String> {
    if input.len() > 8_192 {
        return Err("The pairing link is too long.".into());
    }
    let mut url = Url::parse(input.trim()).map_err(|_| "Paste a complete T3 Code pairing link.")?;
    let params: Vec<_> = url
        .query_pairs()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    let fragments: Vec<_> = url
        .fragment()
        .map(|fragment| reqwest::Url::parse(&format!("https://placeholder.invalid/?{fragment}")))
        .transpose()
        .map_err(|_| "Invalid pairing link.")?
        .map(|fragment| {
            fragment
                .query_pairs()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let token = fragments
        .iter()
        .chain(&params)
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.trim().to_string())
        .filter(|token| !token.is_empty() && token.len() <= 4_096)
        .ok_or("This link has no pairing token. Create a fresh link in T3 Code.")?;
    if let Some((_, host)) = params.iter().find(|(key, _)| key == "host") {
        let host = host.trim_start_matches('/');
        url = Url::parse(&if host.contains("://") {
            host.to_string()
        } else {
            format!("https://{host}")
        })
        .map_err(|_| "Invalid T3 environment address.")?;
    }
    if matches!(url.scheme(), "ws" | "wss") {
        let scheme = if url.scheme() == "ws" {
            "http"
        } else {
            "https"
        };
        url.set_scheme(scheme)
            .map_err(|_| "Invalid pairing link.")?;
    }
    validate_base_url(&url)?;
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Ok(PairingTarget {
        base_url: url,
        credential: token,
    })
}

pub fn validate_base_url(url: &Url) -> Result<(), String> {
    let local = url.host_str().is_some_and(super::custom::is_local_host);
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(
            "Use an HTTP or HTTPS pairing link without username/password credentials.".into(),
        );
    }
    if url.scheme() == "http" && !local {
        return Err(
            "Public T3 addresses require HTTPS. Localhost and private LAN addresses can use HTTP."
                .into(),
        );
    }
    Ok(())
}

fn client() -> Result<Client, String> {
    Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not prepare the T3 connection.".into())
}

async fn bounded_json(mut response: reqwest::Response) -> Result<Value, String> {
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(
            "T3 pairing expired or lacks permission. Reconnect with a fresh pairing link.".into(),
        );
    }
    if !response.status().is_success() {
        return Err(
            "T3 could not complete the request. Check its connection and try again.".into(),
        );
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read the T3 response.")?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE {
            return Err("The T3 skill response exceeds the supported size.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "T3 returned an unsupported response.".into())
}

pub async fn descriptor(base: &Url) -> Result<Value, String> {
    validate_base_url(base)?;
    let result = bounded_json(
        client()?
            .get(
                base.join(".well-known/t3/environment")
                    .map_err(|_| "Invalid T3 address.")?,
            )
            .send()
            .await
            .map_err(|_| "Cannot reach T3. Check that its pairing address is reachable.")?,
    )
    .await?;
    let version = result
        .get("serverVersion")
        .and_then(Value::as_str)
        .ok_or("T3 did not report its version.")?;
    if !t3_skills::supported_version(version) {
        return Err(format!(
            "Requires T3 Code {} or newer.",
            t3_skills::MIN_T3_VERSION
        ));
    }
    if result
        .get("orchestrationProtocolVersion")
        .and_then(Value::as_u64)
        .is_some_and(|version| version != 2)
    {
        return Err(
            "This T3 protocol version is not supported. Update Verenu before pairing.".into(),
        );
    }
    Ok(result)
}

pub async fn exchange(target: &PairingTarget) -> Result<(String, u64), String> {
    let result = bounded_json(
        client()?
            .post(
                target
                    .base_url
                    .join("oauth/token")
                    .map_err(|_| "Invalid T3 address.")?,
            )
            .form(&[
                (
                    "grant_type",
                    "urn:ietf:params:oauth:grant-type:token-exchange",
                ),
                ("subject_token", target.credential.as_str()),
                (
                    "subject_token_type",
                    "urn:t3:params:oauth:token-type:environment-bootstrap",
                ),
                (
                    "requested_token_type",
                    "urn:ietf:params:oauth:token-type:access_token",
                ),
                ("scope", "orchestration:read"),
                ("client_label", "Verenu skill integration"),
                ("client_device_type", "desktop"),
            ])
            .send()
            .await
            .map_err(|_| "Cannot exchange the T3 pairing link.")?,
    )
    .await?;
    if result.get("token_type").and_then(Value::as_str) != Some("Bearer") {
        return Err("This T3 connection requires an unsupported authentication method.".into());
    }
    let token = result
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| token.len() <= 8_192)
        .ok_or("T3 did not issue a credential.")?
        .to_string();
    let expires = result
        .get("expires_in")
        .and_then(Value::as_u64)
        .ok_or("T3 did not report credential expiry.")?;
    Ok((token, expires))
}

pub async fn fetch_catalogs(
    base: &Url,
    token: &str,
    environment_id: &str,
    refresh: bool,
) -> Result<Vec<Catalog>, String> {
    tokio::time::timeout(TIMEOUT, async {
        let ticket = bounded_json(client()?.post(base.join("api/auth/websocket-ticket").map_err(|_| "Invalid T3 address.")?)
            .bearer_auth(token).send().await.map_err(|_| "Cannot authenticate the T3 skill pull.")?).await?;
        let mut url = base.join("ws").map_err(|_| "Invalid T3 address.")?;
        url.set_scheme(if base.scheme() == "https" { "wss" } else { "ws" }).map_err(|_| "Invalid T3 address.")?;
        url.query_pairs_mut().append_pair("wsTicket", ticket.get("ticket").and_then(Value::as_str).ok_or("T3 did not issue a connection ticket.")?);
        url.query_pairs_mut().append_pair("orchestrationProtocol", "2");
        let config = WebSocketConfig::default().max_message_size(Some(MAX_RESPONSE)).max_frame_size(Some(MAX_RESPONSE));
        let (mut socket, _) = tokio_tungstenite::connect_async_with_config(url.as_str(), Some(config), false).await
            .map_err(|_| "Could not open the T3 skill connection. Check the T3 version and pairing address.")?;
        let requests: &[(&str, &str)] = if refresh { &[("1", "server.refreshProviders"), ("2", "server.getConfig")] } else { &[("2", "server.getConfig")] };
        let mut config_value = None;
        for (id, method) in requests {
            socket.send(Message::Text(json!({"_tag":"Request","id":id,"tag":method,"payload":{},"headers":[]}).to_string().into())).await.map_err(|_| "Could not request T3 skills.")?;
            loop {
                let message = socket.next().await.ok_or("T3 closed the skill connection.")?.map_err(|_| "T3 interrupted the skill pull.")?;
                let Message::Text(text) = message else { continue; };
                let response: Value = serde_json::from_str(&text).map_err(|_| "T3 returned unsupported RPC data.")?;
                if response.get("_tag").and_then(Value::as_str) == Some("Ping") {
                    socket.send(Message::Text(json!({"_tag":"Pong"}).to_string().into())).await.map_err(|_| "T3 connection interrupted.")?;
                    continue;
                }
                if response.get("requestId").and_then(Value::as_str) != Some(id) { continue; }
                if response.pointer("/exit/_tag").and_then(Value::as_str) != Some("Success") { return Err("T3 could not read skill metadata. Check provider availability and pairing permissions.".into()); }
                if *method == "server.getConfig" { config_value = response.pointer("/exit/value").cloned(); }
                break;
            }
        }
        let _ = socket.close(None).await;
        parse_catalogs(&config_value.ok_or("T3 did not return its skill catalog.")?, environment_id)
    }).await.map_err(|_| "T3 skill pull timed out. Your previous catalog is retained.".to_string())?
}

pub fn parse_catalogs(config: &Value, environment_id: &str) -> Result<Vec<Catalog>, String> {
    let providers = config
        .get("providers")
        .and_then(Value::as_array)
        .ok_or("This T3 version does not expose provider skills.")?;
    if providers.len() > 64 {
        return Err("Too many T3 providers.".into());
    }
    let mut catalogs = Vec::new();
    for provider in providers {
        if provider.get("enabled").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let Some(instance) = provider.get("instanceId").and_then(Value::as_str) else {
            continue;
        };
        let label = provider
            .get("displayName")
            .or_else(|| provider.get("name"))
            .and_then(Value::as_str)
            .unwrap_or(instance);
        let mut scopes = vec![(
            config.get("cwd").and_then(Value::as_str).unwrap_or(""),
            provider.get("skills"),
        )];
        if let Some(workspaces) = provider.get("workspaceSnapshots").and_then(Value::as_array) {
            if workspaces.len() > 128 {
                return Err("Too many T3 workspaces.".into());
            }
            scopes.extend(workspaces.iter().filter_map(|workspace| {
                Some((workspace.get("cwd")?.as_str()?, workspace.get("skills")))
            }));
        }
        let mut seen_scopes = HashSet::new();
        // Workspace-specific catalogs take precedence over the provider default.
        for (cwd, skills) in scopes.into_iter().rev() {
            if !seen_scopes.insert(cwd) {
                continue;
            }
            let Some(skills) = skills.and_then(Value::as_array) else {
                continue;
            };
            if skills.len() > t3_skills::MAX_SKILLS {
                return Err("The T3 catalog contains too many skills.".into());
            }
            let mut seen = HashSet::new();
            let skills: Vec<Skill> = skills
                .iter()
                .filter(|skill| {
                    skill.get("enabled").and_then(Value::as_bool) == Some(true)
                        && skill.get("userInvocable").and_then(Value::as_bool) != Some(false)
                })
                .filter_map(|skill| {
                    let name = skill.get("name")?.as_str()?.to_string();
                    if !seen.insert(name.to_ascii_lowercase()) {
                        return None;
                    }
                    Some(Skill {
                        name,
                        display_name: skill
                            .get("displayName")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        description: skill
                            .get("shortDescription")
                            .or_else(|| skill.get("description"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect();
            let id = format!("{:x}", Sha256::digest(format!("{instance}\0{cwd}")));
            let revision = format!(
                "{:x}",
                Sha256::digest(
                    serde_json::to_vec(&skills).map_err(|_| "Cannot read skill metadata.")?
                )
            );
            let catalog = Catalog {
                id,
                label: format!("{label} · {}", if cwd.is_empty() { "Default" } else { cwd }),
                environment_id: environment_id.to_string(),
                provider_instance_id: instance.to_string(),
                workspace_id: if cwd.is_empty() {
                    "default".into()
                } else {
                    cwd.to_string()
                },
                revision,
                skills,
            };
            if !t3_skills::valid_catalog(&catalog) {
                return Err("T3 returned invalid skill metadata.".into());
            }
            catalogs.push(catalog);
        }
    }
    Ok(catalogs)
}

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;
