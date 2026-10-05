//! Advisory model capabilities and explicit, captured per-request preferences.
//! No reasoning text is requested for display or persisted here.
use crate::{ollama, types::ProviderProfile};
use genai::chat::ReasoningEffort;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Policy {
    pub state: String,
    pub on: Option<Value>,
    pub off: Option<Value>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            state: "managed".into(),
            on: None,
            off: None,
        }
    }
}
impl Policy {
    pub fn effort(&self, requested: bool) -> Option<ReasoningEffort> {
        let value = match self.state.as_str() {
            "switchable" => {
                if requested {
                    &self.on
                } else {
                    &self.off
                }
            }
            "fixed_on" => &self.on,
            "fixed_off" => &self.off,
            _ => return None,
        };
        value.as_ref().and_then(|v| {
            v.as_u64()
                .map(|v| ReasoningEffort::Budget(v as u32))
                .or_else(|| v.as_str().and_then(ReasoningEffort::from_keyword))
        })
    }
}
pub fn hosted(p: &ProviderProfile) -> Policy {
    if p.service != "llm" {
        return Policy::default();
    }
    let rules: Value =
        serde_json::from_str(include_str!("../../../assets/thinking-models.json")).unwrap();
    let model = p.model.to_ascii_lowercase();
    rules
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            r["protocols"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str() == Some(&p.protocol))
                && (r["prefixes"].as_array().is_some_and(|prefixes| {
                    prefixes
                        .iter()
                        .any(|v| model.starts_with(v.as_str().unwrap()))
                }) || r["models"].as_array().is_some_and(|names| {
                    names.iter().any(|v| {
                        let name = v.as_str().unwrap();
                        // Dated snapshots share capability, sibling Pro/Codex models do not.
                        model == name
                            || model.strip_prefix(name).is_some_and(|suffix| {
                                let b = suffix.as_bytes();
                                b.len() == 11
                                    && b[0] == b'-'
                                    && b[5] == b'-'
                                    && b[8] == b'-'
                                    && b.iter()
                                        .enumerate()
                                        .all(|(i, c)| [0, 5, 8].contains(&i) || c.is_ascii_digit())
                            })
                    })
                }))
        })
        .map(|r| serde_json::from_value(r.clone()).unwrap())
        .unwrap_or_default()
}
pub fn local(metadata: Option<&Value>, capabilities: &[String]) -> Policy {
    let Some(values) = metadata
        .and_then(|v| v.get("values"))
        .and_then(Value::as_array)
    else {
        return if capabilities.iter().any(|v| v == "thinking") {
            Policy::default()
        } else {
            Policy {
                state: "fixed_off".into(),
                on: None,
                off: None,
            }
        };
    };
    let off = values.contains(&json!(false)) || values.contains(&json!("none"));
    let on = if values.contains(&json!(true)) {
        Some(json!("low"))
    } else {
        ["minimal", "low", "medium", "high"]
            .iter()
            .find(|v| values.contains(&json!(v)))
            .map(|v| json!(v))
    };
    Policy {
        state: match (off, on.is_some()) {
            (true, true) => "switchable",
            (true, false) => "fixed_off",
            (false, true) => "fixed_on",
            _ => "managed",
        }
        .into(),
        on,
        off: off.then(|| json!("none")),
    }
}
pub async fn capability(p: &ProviderProfile) -> Policy {
    if p.protocol != "ollama" {
        return hosted(p);
    }
    ollama::details(&p.endpoint, &p.model, false)
        .await
        .model
        .map(|m| m.thinking)
        .unwrap_or_default()
}

/// Capture the user's preference independently of capability restrictions.
/// Existing jobs keep their already-captured policy; new policies also give
/// translation caches a distinct identity when a former default is overridden.
pub async fn resolve(p: &ProviderProfile) -> Policy {
    if p.service != "llm" {
        return Policy::default();
    }
    let capability = capability(p).await;
    Policy {
        state: "switchable".into(),
        on: Some(capability.on.unwrap_or_else(|| json!("low"))),
        // genai maps None to omission for Gemini, which leaves thinking enabled
        // by default. A zero budget is the actual request to disable thinking.
        // Anthropic's None omits its opt-in thinking configuration.
        off: Some(if p.protocol == "gemini" {
            json!(0)
        } else {
            json!("none")
        }),
    }
}
