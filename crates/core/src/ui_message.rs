//! Language-independent messages at storage and IPC boundaries. Processing continues to use
//! canonical diagnostic text; serialized records carry stable catalog identities and parameters.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::LazyLock};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UiMessage {
    pub key: String,
    pub args: BTreeMap<String, String>,
    pub fallback: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Deserialize)]
struct Pattern {
    key: String,
    parts: Vec<String>,
    slots: Vec<String>,
}
static EN: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../resources/i18n/en.json"))
        .expect("bundled English catalog")
});
static ZH: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../resources/i18n/zh-Hans.json"))
        .expect("bundled Chinese catalog")
});
static EXACT: LazyLock<BTreeMap<&'static str, &'static str>> = LazyLock::new(|| {
    EN.iter()
        .map(|(key, text)| (text.as_str(), key.as_str()))
        .collect()
});
static PATTERNS: LazyLock<Vec<Pattern>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../resources/i18n/patterns.json"))
        .expect("bundled message patterns")
});

impl UiMessage {
    pub fn error_text(text: impl Into<String>) -> Self {
        let message = Self::from_text(text);
        if message.key != "diagnostic" {
            return message;
        }
        let mut summary = Self::from_text("Unable to complete this operation.");
        summary.detail = Some(message.fallback.clone());
        summary.fallback = message.fallback;
        summary
    }
    pub fn from_text(text: impl Into<String>) -> Self {
        let text = text.into();
        if let Some(key) = EXACT.get(text.as_str()) {
            return Self {
                key: (*key).into(),
                args: BTreeMap::new(),
                fallback: text,
                detail: None,
            };
        }
        for p in PATTERNS.iter() {
            if let Some(args) = capture(p, &text) {
                return Self {
                    key: p.key.clone(),
                    args,
                    fallback: text,
                    detail: None,
                };
            }
        }
        // Do not guess the meaning of provider/OS output or send it to a translation service.
        Self {
            key: "diagnostic".into(),
            args: BTreeMap::new(),
            fallback: text,
            detail: None,
        }
    }
    pub fn render(&self, chinese: bool) -> String {
        let catalog = if chinese { &*ZH } else { &*EN };
        let template = catalog
            .get(&self.key)
            .cloned()
            .unwrap_or_else(|| self.fallback.clone());
        let mut text = String::new();
        let mut rest = template.as_str();
        while let Some(start) = rest.find('{') {
            text.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('}').map(|n| start + n) else {
                text.push_str(&rest[start..]);
                rest = "";
                break;
            };
            let placeholder = &rest[start..=end];
            text.push_str(
                self.args
                    .get(&rest[start + 1..end])
                    .map(String::as_str)
                    .unwrap_or(placeholder),
            );
            rest = &rest[end + 1..];
        }
        text.push_str(rest);
        if let Some(detail) = &self.detail {
            text.push('\n');
            text.push_str(detail);
        }
        text
    }
}
fn capture(p: &Pattern, text: &str) -> Option<BTreeMap<String, String>> {
    let mut rest = text.strip_prefix(p.parts.first()?.as_str())?;
    let mut args = BTreeMap::new();
    for (i, slot) in p.slots.iter().enumerate() {
        let next = p.parts.get(i + 1)?;
        let n = if next.is_empty() {
            rest.len()
        } else if i + 1 == p.slots.len() {
            rest.strip_suffix(next)?.len()
        } else {
            rest.find(next)?
        };
        args.insert(slot.clone(), rest[..n].into());
        rest = rest.get(n + next.len()..)?;
    }
    rest.is_empty().then_some(args)
}
impl From<String> for UiMessage {
    fn from(text: String) -> Self {
        Self::from_text(text)
    }
}
impl From<&str> for UiMessage {
    fn from(text: &str) -> Self {
        Self::from_text(text)
    }
}
impl std::fmt::Display for UiMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.fallback)
    }
}
impl std::error::Error for UiMessage {}

// String-facing processing APIs keep their established semantics. No locale is captured by
// jobs or included in render/translation hashes. Fresh-format files store UiMessage objects.
pub mod text {
    use super::*;
    pub fn serialize<S: serde::Serializer>(text: &str, s: S) -> Result<S::Ok, S::Error> {
        UiMessage::from_text(text).serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        Ok(UiMessage::deserialize(d)?.fallback)
    }
}
pub mod optional {
    use super::*;
    pub fn serialize<S: serde::Serializer>(text: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
        text.as_ref()
            .map(|s| UiMessage::from_text(s.clone()))
            .serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        Ok(Option::<UiMessage>::deserialize(d)?.map(|m| m.fallback))
    }
}
pub mod optional_error {
    use super::*;
    pub fn serialize<S: serde::Serializer>(text: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
        text.as_ref()
            .map(|s| UiMessage::error_text(s.clone()))
            .serialize(s)
    }
    pub use super::optional::deserialize;
}
pub mod list {
    use super::*;
    pub fn serialize<S: serde::Serializer>(text: &[String], s: S) -> Result<S::Ok, S::Error> {
        text.iter()
            .map(|v| UiMessage::from_text(v.clone()))
            .collect::<Vec<_>>()
            .serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
        Ok(Vec::<UiMessage>::deserialize(d)?
            .into_iter()
            .map(|m| m.fallback)
            .collect())
    }
}
pub mod map {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        text: &BTreeMap<String, String>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        text.iter()
            .map(|(k, v)| (k.clone(), UiMessage::from_text(v.clone())))
            .collect::<BTreeMap<_, _>>()
            .serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<String, String>, D::Error> {
        Ok(BTreeMap::<String, UiMessage>::deserialize(d)?
            .into_iter()
            .map(|(k, m)| (k, m.fallback))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structured_messages_roundtrip_without_capturing_a_locale() {
        let msg = UiMessage::from_text("Provider returned no text");
        assert_ne!(msg.key, "diagnostic");
        assert_eq!(msg.render(false), "Provider returned no text");
        assert_eq!(msg.render(true), "服务未返回文字");
        assert_eq!(
            serde_json::from_str::<UiMessage>(&serde_json::to_string(&msg).unwrap()).unwrap(),
            msg
        );
    }
    #[test]
    fn external_diagnostics_are_not_rewritten() {
        let msg = UiMessage::from_text("Remote error XYZ; custom model → 私有模型");
        assert_eq!(msg.render(true), msg.fallback);
        let error = UiMessage::error_text(msg.fallback.clone());
        assert_eq!(
            error.render(true),
            format!("无法完成此操作。\n{}", msg.fallback)
        );
        assert_eq!(error.fallback, msg.fallback);
    }
    #[test]
    fn counts_are_parameters_and_catalogs_are_complete() {
        assert_eq!(EN.keys().collect::<Vec<_>>(), ZH.keys().collect::<Vec<_>>());
        let msg = UiMessage::from_text("8 regions saved");
        assert_eq!(msg.render(true), "已保存 8 个区域");
        assert_eq!(msg.args.get("arg0").unwrap(), "8");
    }
}
