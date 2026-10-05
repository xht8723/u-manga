//! Complete editable prompts; generated input and protocol validation remain application-owned.
use crate::{
    prompts::{self, TranslationPrompt},
    types::*,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

pub const MAX_BYTES: usize = 16 * 1024;
pub const SIMPLE_PROMPT_VERSION: u32 = 4;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Task {
    SimpleTextTranslation,
    TextTranslation,
    VisionTranslation,
    GlossaryDetection,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Overrides {
    pub simple_text_translation: Option<String>,
    pub text_translation: Option<String>,
    pub vision_translation: Option<String>,
    pub glossary_detection: Option<String>,
}
impl Overrides {
    pub fn get(&self, task: Task) -> Option<&str> {
        match task {
            Task::SimpleTextTranslation => &self.simple_text_translation,
            Task::TextTranslation => &self.text_translation,
            Task::VisionTranslation => &self.vision_translation,
            Task::GlossaryDetection => &self.glossary_detection,
        }
        .as_deref()
    }
    pub fn validate(&self) -> Result<()> {
        for text in [
            &self.simple_text_translation,
            &self.text_translation,
            &self.vision_translation,
            &self.glossary_detection,
        ]
        .into_iter()
        .flatten()
        {
            validate_template(text)?;
        }
        Ok(())
    }
}
fn catalog() -> &'static Value {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../../../resources/instructions.json"))
            .expect("bundled instruction catalog")
    })
}
fn name(task: Task) -> &'static str {
    match task {
        Task::SimpleTextTranslation => "simpleTextTranslation",
        Task::TextTranslation => "textTranslation",
        Task::VisionTranslation => "visionTranslation",
        Task::GlossaryDetection => "glossaryDetection",
    }
}
pub fn default_template(task: Task, _source: &str) -> String {
    catalog()[name(task)].as_str().unwrap().to_owned()
}
pub fn defaults(source: &str) -> Overrides {
    Overrides {
        simple_text_translation: Some(default_template(Task::SimpleTextTranslation, source)),
        text_translation: Some(default_template(Task::TextTranslation, source)),
        vision_translation: Some(default_template(Task::VisionTranslation, source)),
        glossary_detection: Some(default_template(Task::GlossaryDetection, source)),
    }
}
const VARIABLES: [&str; 4] = [
    "source_language",
    "source_language_code",
    "target_language",
    "target_language_code",
];
// Only {{identifier}} is template syntax; ordinary JSON/braces are literal data.
fn substitute(text: &str, mut value: impl FnMut(&str) -> Result<String>) -> Result<String> {
    let mut out = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("{{") {
        out.push_str(&remaining[..start]);
        remaining = &remaining[start..];
        if let Some(end) = remaining[2..].find("}}") {
            let key = &remaining[2..end + 2];
            if !key.is_empty() && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
                out.push_str(&value(key)?);
                remaining = &remaining[end + 4..];
                continue;
            }
        }
        out.push_str("{{");
        remaining = &remaining[2..];
    }
    out.push_str(remaining);
    Ok(out)
}
pub fn validate_template(text: &str) -> Result<()> {
    ensure!(
        text.len() <= MAX_BYTES,
        "Prompts exceed the 16 KiB UTF-8 limit."
    );
    substitute(text, |key| {
        ensure!(
            VARIABLES.contains(&key),
            "Unknown prompt placeholder: {key}"
        );
        Ok(String::new())
    })?;
    Ok(())
}
pub fn expand(text: &str, settings: &TranslationSettings) -> Result<String> {
    substitute(text, |key| {
        Ok(match key {
            "source_language" => prompts::language_name(&settings.source_language),
            "source_language_code" => &settings.source_language,
            "target_language" => prompts::language_name(&settings.target_language),
            "target_language_code" => &settings.target_language,
            _ => anyhow::bail!("Unknown prompt placeholder: {key}"),
        }
        .to_owned())
    })
}
pub fn effective(
    overrides: &Overrides,
    settings: &TranslationSettings,
    task: Task,
) -> Result<String> {
    let template = overrides
        .get(task)
        .map(str::to_owned)
        .unwrap_or_else(|| default_template(task, &settings.source_language));
    validate_template(&template)?;
    expand(&template, settings)
}
/// Freeze defaults into the provider snapshot. Captured languages resolve placeholders later.
pub fn capture(
    provider: &ProviderProfile,
    settings: &TranslationSettings,
) -> Result<ProviderProfile> {
    provider.instructions.validate()?;
    let mut captured = provider.clone();
    let built_in = defaults(&settings.source_language);
    captured.instructions = Overrides {
        simple_text_translation: provider
            .instructions
            .simple_text_translation
            .clone()
            .or(built_in.simple_text_translation),
        text_translation: provider
            .instructions
            .text_translation
            .clone()
            .or(built_in.text_translation),
        vision_translation: provider
            .instructions
            .vision_translation
            .clone()
            .or(built_in.vision_translation),
        glossary_detection: provider
            .instructions
            .glossary_detection
            .clone()
            .or(built_in.glossary_detection),
    };
    Ok(captured)
}
pub fn simple_translation(provider: &ProviderProfile, has_image: bool) -> bool {
    provider.service == "llm" && provider.simple_translation && !has_image
}

/// Reject only explicit redaction-only source, never general punctuation or symbols.
pub fn simple_source_readable(source: &str) -> bool {
    let mut blocks = 0;
    for c in source.chars().filter(|c| !c.is_whitespace()) {
        if !matches!(c, '■' | '█') {
            return true;
        }
        blocks += 1;
    }
    blocks == 1
}

pub fn translation_task(provider: &ProviderProfile, settings: &TranslationSettings) -> Task {
    if settings.mode == "vision" {
        Task::VisionTranslation
    } else if simple_translation(provider, false) {
        Task::SimpleTextTranslation
    } else {
        Task::TextTranslation
    }
}
pub fn extraction_identity(
    provider: &ProviderProfile,
    settings: &TranslationSettings,
) -> Result<String> {
    // Preserve the extraction identity's established field order/shape. New
    // translation-only options must not invalidate glossary extraction caches.
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ExtractionTransport<'a> {
        instructions: ExtractionInstructionSlots,
        thinking: bool,
        thinking_policy: &'a Option<crate::thinking::Policy>,
        id: &'a str,
        name: &'a str,
        service: &'a str,
        protocol: &'a str,
        endpoint: &'a str,
        model: &'a str,
        vision: bool,
        region: &'a str,
        app_id: &'a str,
        rate_limit: f32,
    }
    #[derive(Serialize, Default)]
    #[serde(rename_all = "camelCase")]
    struct ExtractionInstructionSlots {
        text_translation: Option<String>,
        vision_translation: Option<String>,
        glossary_detection: Option<String>,
    }
    let transport = ExtractionTransport {
        instructions: ExtractionInstructionSlots::default(),
        thinking: provider.thinking,
        thinking_policy: &provider.thinking_policy,
        id: &provider.id,
        name: &provider.name,
        service: &provider.service,
        protocol: &provider.protocol,
        endpoint: &provider.endpoint,
        model: &provider.model,
        vision: provider.vision,
        region: &provider.region,
        app_id: &provider.app_id,
        rate_limit: provider.rate_limit,
    };
    Ok(crate::store::digest(&serde_json::to_vec(&(
        transport,
        effective(&provider.instructions, settings, Task::GlossaryDetection)?,
    ))?))
}
fn glossary_text(entries: &[GlossaryEntry]) -> Result<String> {
    entries
        .iter()
        .map(|entry| {
            Ok(format!(
                "{}: {}",
                serde_json::to_string(&entry.source)?,
                serde_json::to_string(&entry.target)?
            ))
        })
        .collect::<Result<Vec<_>>>()
        .map(|lines| lines.join("\n"))
}

fn references(settings: &TranslationSettings, task: Task, context: &str) -> Result<Vec<String>> {
    let mut sections = Vec::new();
    if task == Task::GlossaryDetection {
        return Ok(sections);
    }
    if settings.glossary_enabled && !settings.glossary.is_empty() {
        sections.push(format!(
            "Glossary:\nUse these preferred translations when applicable.\n{}",
            glossary_text(&settings.glossary)?
        ));
    }
    if !context.is_empty() {
        sections.push(format!(
            "Use previous dialogue as context; translate only current text.\n{context}"
        ));
    }
    Ok(sections)
}

pub fn compose(
    overrides: &Overrides,
    settings: &TranslationSettings,
    task: Task,
    context: &str,
    _schema: Option<&Value>,
) -> Result<TranslationPrompt> {
    if task == Task::SimpleTextTranslation {
        return Ok(TranslationPrompt {
            system: effective(overrides, settings, task)?,
            user_preamble: simple_preamble(settings, context, false)?,
        });
    }
    let mut sections = references(settings, task, context)?;
    sections.push(
        if task == Task::GlossaryDetection {
            "Current source texts:"
        } else {
            "Current regions to translate:"
        }
        .into(),
    );
    Ok(TranslationPrompt {
        system: effective(overrides, settings, task)?,
        user_preamble: sections.join("\n\n"),
    })
}

/// Separate Simple system/user content, shared by providers, preview and service tests.
pub fn simple_prompt(
    overrides: &Overrides,
    settings: &TranslationSettings,
    context: &str,
    regions: &[Region],
) -> Result<TranslationPrompt> {
    crate::text_batches::validate_request(regions, true)?;
    Ok(TranslationPrompt {
        system: effective(overrides, settings, Task::SimpleTextTranslation)?,
        user_preamble: format!(
            "{}\n{}",
            simple_preamble(settings, context, regions.len() > 1)?,
            crate::text_batches::current_text(regions, true)?
        ),
    })
}

fn simple_preamble(
    settings: &TranslationSettings,
    context: &str,
    multiple: bool,
) -> Result<String> {
    let mut sections = references(settings, Task::SimpleTextTranslation, context)?;
    sections.push(
        if multiple {
            "Current texts:"
        } else {
            "Current text:"
        }
        .into(),
    );
    Ok(sections.join("\n\n"))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub prompt: String,
    pub system: String,
    pub user: String,
    pub schema: Option<Value>,
}
/// Pure synthetic preview; no credentials, network or user files.
pub fn preview(
    provider: &ProviderProfile,
    settings: &TranslationSettings,
    task: Task,
) -> Result<Preview> {
    provider.instructions.validate()?;
    let mut sample = settings.clone();
    sample.glossary_enabled = true;
    sample.glossary = vec![GlossaryEntry {
        source: "Sample name".into(),
        target: "Preferred name".into(),
    }];
    let region = Region {
        id: "sample-region".into(),
        source: "123".into(),
        target: "456".into(),
        ..Default::default()
    };
    let schema = (crate::providers::is_ollama(provider)
        && !matches!(task, Task::SimpleTextTranslation | Task::GlossaryDetection))
    .then(|| crate::providers::translation_schema(std::slice::from_ref(&region)));
    let composed = compose(
        &provider.instructions,
        &sample,
        task,
        "Page 1:\n456",
        schema.as_ref(),
    )?;
    let mut user = if task == Task::SimpleTextTranslation {
        let regions = [
            region.clone(),
            Region {
                id: "sample-region-2".into(),
                source: "456".into(),
                ..Default::default()
            },
            Region {
                id: "sample-region-3".into(),
                source: "789".into(),
                ..Default::default()
            },
        ];
        simple_prompt(&provider.instructions, &sample, "Page 1:\n456", &regions)?.user_preamble
    } else {
        format!(
            "{}\n\n{}",
            composed.user_preamble,
            if task == Task::GlossaryDetection {
                prompts::glossary_sources(std::slice::from_ref(&region))?
            } else {
                prompts::region_label(&region)?
            }
        )
    };
    if task == Task::VisionTranslation {
        user.push_str("\n[Sample image crop / overlapping tile attachment]");
    }
    Ok(Preview {
        prompt: effective(&provider.instructions, &sample, task)?,
        system: composed.system,
        user,
        schema,
    })
}
