//! Metadata-only discovery for an existing Ollama server. Never pulls or loads models.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

pub const DEFAULT_SERVER: &str = "http://localhost:11434";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    pub name: String,
    pub size: u64,
    pub digest: String,
    pub remote: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    #[serde(flatten)]
    pub installed: InstalledModel,
    pub capabilities: Vec<String>,
    pub thinking: crate::thinking::Policy,
}
impl ModelInfo {
    pub fn validate(&self, vision: bool) -> Result<()> {
        if self.installed.remote {
            bail!(
                "Choose a model installed on this Ollama server. Cloud-backed models are not supported here."
            );
        }
        if !self.capabilities.iter().any(|c| c == "completion") {
            bail!(
                "This Ollama model does not report text-generation support. Choose a chat model or update Ollama."
            );
        }
        if vision && !self.capabilities.iter().any(|c| c == "vision") {
            bail!("This Ollama model cannot read images. Choose a vision model or Local OCR.");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelList {
    pub endpoint: String,
    pub connected: bool,
    pub models: Vec<InstalledModel>,
    #[serde(with = "crate::ui_message::optional")]
    pub message: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub endpoint: String,
    pub connected: bool,
    pub model: Option<ModelInfo>,
    #[serde(with = "crate::ui_message::optional")]
    pub message: Option<String>,
}

pub fn base_url(endpoint: &str) -> Result<String> {
    let input = if endpoint.trim().is_empty() {
        DEFAULT_SERVER
    } else {
        endpoint.trim()
    };
    let mut url = url::Url::parse(input)?;
    if !["http", "https"].contains(&url.scheme()) || url.host_str().is_none() {
        bail!("Enter an HTTP or HTTPS Ollama server address.");
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Use a server address without credentials, query parameters, or a fragment.");
    }
    let path = url.path().trim_end_matches('/');
    let path = path
        .strip_suffix("/v1")
        .or_else(|| path.strip_suffix("/api"))
        .unwrap_or(path)
        .to_owned();
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_owned())
}
pub fn destination(endpoint: &str) -> String {
    let Ok(base) = base_url(endpoint) else {
        return endpoint.to_owned();
    };
    let host = url::Url::parse(&base)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    let local = host == "localhost"
        || host == "[::1]"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    format!(
        "{} · {base}",
        if local {
            "This computer"
        } else {
            "Ollama server"
        }
    )
}

type Cached = Option<(Instant, std::result::Result<Value, String>)>;
type Cell = Arc<Mutex<Cached>>;
static METADATA: OnceLock<Mutex<HashMap<String, Cell>>> = OnceLock::new();
async fn metadata(
    base: &str,
    route: &str,
    body: Option<Value>,
    identity: &str,
    force: bool,
) -> Result<Value> {
    let started = Instant::now();
    let key = format!("{base}/{route}:{identity}");
    let cell = {
        let mut cache = METADATA.get_or_init(Mutex::default).lock().await;
        // Bound retained metadata. In-flight cells remain owned by their callers.
        if cache.len() > 256 {
            cache.retain(|_, c| Arc::strong_count(c) > 1);
        }
        cache
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone()
    };
    // Only this metadata key is serialized; no scheduler or global lock spans I/O.
    let mut cached = cell.lock().await;
    if let Some((at, result)) = &*cached {
        let ttl = if result.is_ok() { 30 } else { 2 };
        if (*at >= started || !force) && at.elapsed() < Duration::from_secs(ttl) {
            return result.clone().map_err(anyhow::Error::msg);
        }
    }
    let result: Result<Value> = async {
        let client = crate::http_transport::client(crate::http_transport::Transport::OllamaMetadata)?;
        let request = if let Some(body) = body { client.post(format!("{base}/{route}")).json(&body) }
            else { client.get(format!("{base}/{route}")) };
        let response = request.send().await.map_err(|e| anyhow::anyhow!(if e.is_timeout() {
            "Ollama did not respond within 5 seconds. Check the server address and try Refresh."
        } else { "Cannot connect to Ollama. Start Ollama and check the server address and network access." }))?;
        if !response.status().is_success() {
            bail!("Ollama metadata request failed (HTTP {}). Check the model, server address, and Ollama version.", response.status().as_u16());
        }
        let bytes = crate::http_transport::bounded_body(response, 4 * 1024 * 1024).await?;
        serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Ollama returned invalid model metadata."))
    }.await;
    *cached = Some((
        Instant::now(),
        result
            .as_ref()
            .map(Clone::clone)
            .map_err(ToString::to_string),
    ));
    result
}
fn remote(value: &Value, name: &str) -> bool {
    ["remote_model", "remote_host"].iter().any(|key| {
        value
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty())
    }) || name.ends_with(":cloud")
        || name.ends_with("-cloud")
}
pub async fn list(endpoint: &str, force: bool) -> ModelList {
    let mut result = ModelList {
        endpoint: endpoint.into(),
        connected: false,
        models: vec![],
        message: None,
    };
    let fetched: Result<()> = async {
        let base = base_url(endpoint)?;
        result.endpoint = base.clone();
        let data = metadata(&base, "api/tags", None, "", force).await?;
        let rows = data
            .get("models")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("Ollama returned an invalid model list."))?;
        result.models = rows
            .iter()
            .filter_map(|m| {
                let name = m.get("name").and_then(Value::as_str)?;
                Some(InstalledModel {
                    name: name.into(),
                    size: m["size"].as_u64().unwrap_or(0),
                    digest: m["digest"].as_str().unwrap_or_default().into(),
                    remote: remote(m, name),
                })
            })
            .collect();
        result.models.sort_by(|a, b| a.name.cmp(&b.name));
        result.connected = true;
        Ok(())
    }
    .await;
    if let Err(e) = fetched {
        result.message = Some(e.to_string());
    }
    result
}
pub async fn details(endpoint: &str, model: &str, force: bool) -> ModelStatus {
    let inventory = list(endpoint, force).await;
    let mut result = ModelStatus {
        endpoint: inventory.endpoint,
        connected: inventory.connected,
        model: None,
        message: inventory.message,
    };
    if !result.connected {
        return result;
    }
    let Some(mut installed) = inventory.models.into_iter().find(|m| m.name == model) else {
        result.message = Some(if model.trim().is_empty() {
            "Select an installed Ollama model.".into()
        } else {
            format!(
                "Model '{model}' is not installed on this Ollama server. Install it in Ollama, then Refresh."
            )
        });
        return result;
    };
    let identity = format!("{}:{}", installed.name, installed.digest);
    match metadata(
        &result.endpoint,
        "api/show",
        Some(json!({"model": model, "verbose": false})),
        &identity,
        force,
    )
    .await
    {
        Ok(data) => {
            installed.remote |= remote(&data, model);
            let capabilities: Vec<String> = data
                .get("capabilities")
                .and_then(Value::as_array)
                .map(|v| {
                    v.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            result.model = Some(ModelInfo {
                thinking: crate::thinking::local(data.get("thinking"), &capabilities),
                installed,
                capabilities,
            });
        }
        Err(e) => result.message = Some(e.to_string()),
    }
    result
}
pub async fn require_model(endpoint: &str, model: &str, vision: bool) -> Result<ModelInfo> {
    let status = details(endpoint, model, false).await;
    let info = status.model.ok_or_else(|| {
        anyhow::anyhow!(
            status
                .message
                .unwrap_or_else(|| "Check the Ollama server and selected model.".into())
        )
    })?;
    info.validate(vision)?;
    Ok(info)
}
