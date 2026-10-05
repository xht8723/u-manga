use crate::{inference, ollama, prompts, store, types::*};
use anyhow::{Context, Result, bail};
use base64::Engine;
use genai::{
    Client, ModelIden, ServiceTarget,
    adapter::AdapterKind,
    chat::{
        ChatMessage, ChatOptions, ChatRequest, ChatResponse, ContentPart, JsonSpec, MessageContent,
    },
    resolver::{AuthData, Endpoint, ServiceTargetResolver},
};
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path, time::Duration};

pub fn is_ollama(p: &ProviderProfile) -> bool {
    p.service == "llm" && p.protocol == "ollama"
}
pub fn credential_required(p: &ProviderProfile) -> bool {
    !is_ollama(p)
}

pub fn credential_scope(p: &ProviderProfile) -> Result<String> {
    let destination = match p.service.as_str() {
        "google" => "https://translation.googleapis.com/".into(),
        "microsoft" => "https://api.cognitive.microsofttranslator.com/".into(),
        "deepl" => "https://api.deepl.com/".into(),
        "baidu" => "https://fanyi-api.baidu.com/".into(),
        _ if is_ollama(p) => ollama::base_url(&p.endpoint)?,
        _ => endpoint(p),
    };
    let u = url::Url::parse(&destination)?;
    if !["http", "https"].contains(&u.scheme()) || u.host_str().is_none() {
        bail!("Use an HTTP or HTTPS endpoint")
    }
    if credential_required(p)
        && u.scheme() != "https"
        && u.host_str() != Some("localhost")
        && u.host_str() != Some("127.0.0.1")
        && u.host_str() != Some("[::1]")
    {
        bail!("Credentials require HTTPS except for a local endpoint")
    };
    if !u.username().is_empty() || u.password().is_some() {
        bail!("Do not place credentials in endpoint URLs")
    };
    Ok(format!(
        "{}:{}",
        p.id,
        store::digest(u.origin().ascii_serialization().as_bytes())
    ))
}
pub fn endpoint(p: &ProviderProfile) -> String {
    if is_ollama(p) {
        return format!(
            "{}/v1/",
            ollama::base_url(&p.endpoint).unwrap_or_else(|_| p.endpoint.clone())
        );
    }
    if !p.endpoint.trim().is_empty() {
        return p.endpoint.trim_end_matches('/').to_owned() + "/";
    }
    match p.protocol.as_str() {
        "anthropic" => "https://api.anthropic.com/v1/",
        "gemini" => "https://generativelanguage.googleapis.com/v1beta/",
        _ => "https://api.openai.com/v1/",
    }
    .into()
}

/// Generation constraints for this batch, shared by Local OCR text and Vision.
/// Request one result for every submitted region, including an empty target when
/// needed. Recovery still accepts useful partial results; `validate` rejects
/// invented/repeated IDs, since an enum cannot enforce uniqueness across rows.
pub fn translation_schema(regions: &[Region]) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["regions"],
        "properties": {
            "regions": {
                "type": "array",
                "minItems": regions.len(),
                "maxItems": regions.len(),
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "source", "target"],
                    "properties": {
                        "id": {"type": "string", "enum": regions.iter().map(|r| &r.id).collect::<Vec<_>>()},
                        "source": {"type": "string"},
                        "target": {"type": "string"}
                    }
                }
            }
        }
    })
}

fn response_text<'a>(row: &'a Value, field: &str) -> Result<&'a str> {
    // Preserve the existing partial-response contract for omitted text, but do
    // not silently turn malformed values (numbers, null, arrays) into text.
    match row.get(field) {
        None => Ok(""),
        Some(value) => value
            .as_str()
            .with_context(|| format!("Region {field} must be a string")),
    }
}

pub fn validate(text: &str, expected: &[Region]) -> Result<Vec<TranslationItem>> {
    validate_response(text, expected, false)
}
fn validate_response(
    text: &str,
    expected: &[Region],
    transcription: bool,
) -> Result<Vec<TranslationItem>> {
    let text = text
        .trim()
        .strip_prefix("```json")
        .or_else(|| text.trim().strip_prefix("```"))
        .unwrap_or(text.trim())
        .trim()
        .trim_end_matches("```")
        .trim();
    let value: Value = serde_json::from_str(text).context("Provider returned malformed JSON")?;
    let rows = value
        .get("regions")
        .and_then(Value::as_array)
        .context("Missing regions array")?;
    let ids = expected
        .iter()
        .map(|r| r.id.as_str())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for row in rows {
        let id = row
            .get("id")
            .and_then(Value::as_str)
            .context("Region ID must be a string")?;
        if !ids.contains(id) || !seen.insert(id) {
            bail!("Unknown or duplicate region ID")
        };
        if transcription
            && (!row.get("source").is_some_and(Value::is_string)
                || !row.get("target").is_some_and(Value::is_string))
        {
            bail!("Transcription requires source and target strings");
        }
        let source = response_text(row, "source")?;
        let target = response_text(row, "target")?.trim();
        crate::safety::response_text(source, target)?;
        if transcription && !target.is_empty() {
            bail!("Transcription must not contain a translation");
        }
        if target.is_empty() && !transcription {
            continue;
        }
        items.push(TranslationItem {
            id: id.into(),
            source: source.into(),
            target: target.into(),
            // Geometry and writing direction are exclusively local decisions,
            // even if a provider ignores the schema or supplies extra fields.
            direction: String::new(),
        });
    }
    if transcription && items.len() != expected.len() {
        bail!("Missing transcription result");
    }
    Ok(items)
}

fn final_translation_text<'a>(p: &ProviderProfile, response: &'a ChatResponse) -> Result<&'a str> {
    if is_ollama(p) {
        // Read finish metadata in memory only. Never use reasoning as a final
        // translation, or log the raw response (which contains page content).
        let finish = response
            .captured_raw_body
            .as_ref()
            .and_then(|body| body.pointer("/choices/0/finish_reason"))
            .and_then(Value::as_str);
        if finish == Some("length") {
            bail!(
                "Ollama reached its context or output limit before finishing the translation. Retry the page; if this repeats, increase the context length in Ollama or choose another local model."
            );
        }
        if response
            .first_text()
            .is_none_or(|text| text.trim().is_empty())
        {
            if response
                .reasoning_content
                .as_ref()
                .is_some_and(|text| !text.trim().is_empty())
            {
                bail!(
                    "Ollama returned reasoning but no final translation. Retry the page or choose a model that supports disabling thinking."
                );
            }
            bail!(
                "Ollama returned an empty final answer. Retry the page or choose another local model."
            );
        }
    }
    response.first_text().context("Provider returned no text")
}
pub fn capabilities(p: &ProviderProfile, s: &TranslationSettings) -> (bool, bool, String) {
    match p.service.as_str() {
        "llm" => (true, true, String::new()),
        "deepl" => (
            !s.deepl_glossary_id.is_empty(),
            true,
            "DeepL terminology requires a hosted glossary ID for this language pair".into(),
        ),
        "microsoft" => (
            false,
            false,
            "Microsoft dictionary support is not enabled for this language pair".into(),
        ),
        _ => (
            false,
            false,
            "This service has no request-level glossary or preceding-page context".into(),
        ),
    }
}

pub async fn llm(
    p: &ProviderProfile,
    key: &str,
    s: &TranslationSettings,
    regions: &[Region],
    image: Option<&image::DynamicImage>,
    context: &str,
) -> Result<Vec<TranslationItem>> {
    let simple = crate::instructions::simple_translation(p, image.is_some());
    let readable: Vec<_> = regions
        .iter()
        .filter(|r| {
            if simple {
                crate::instructions::simple_source_readable(&r.source)
            } else {
                image.is_some() || !r.source.trim().is_empty()
            }
        })
        .cloned()
        .collect();
    if readable.is_empty() {
        return Ok(vec![]);
    }
    if image.is_none() {
        crate::text_batches::validate_request(&readable, simple)?;
    }
    let text = llm_request(
        p,
        key,
        s,
        &readable,
        ImageInput::from(image),
        context,
        RequestKind::Translation,
    )
    .await?;
    if simple {
        return crate::text_batches::simple_results(&text, &readable);
    }
    let mut result = validate(&text, &readable)?;
    if image.is_none() {
        for item in &mut result {
            item.source = readable
                .iter()
                .find(|r| r.id == item.id)
                .unwrap()
                .source
                .clone();
        }
    }
    Ok(result)
}

pub async fn transcribe(
    p: &ProviderProfile,
    key: &str,
    s: &TranslationSettings,
    region: &Region,
    image: &image::DynamicImage,
) -> Result<String> {
    let items = transcribe_batch(p, key, s, std::slice::from_ref(region), image).await?;
    Ok(items
        .into_iter()
        .next()
        .context("Missing transcription result")?
        .source)
}
#[derive(Clone, Copy, PartialEq)]
enum RequestKind {
    Translation,
    Transcription,
    Glossary,
}
pub async fn transcribe_batch(
    p: &ProviderProfile,
    key: &str,
    s: &TranslationSettings,
    regions: &[Region],
    image: &image::DynamicImage,
) -> Result<Vec<TranslationItem>> {
    let text = llm_request(
        p,
        key,
        s,
        regions,
        ImageInput::Page(image),
        "",
        RequestKind::Transcription,
    )
    .await?;
    validate_response(&text, regions, true)
}
pub async fn extract_glossary(
    p: &ProviderProfile,
    key: &str,
    s: &TranslationSettings,
    regions: &[Region],
) -> Result<crate::glossary::Extraction> {
    let text = llm_request(
        p,
        key,
        s,
        regions,
        ImageInput::None,
        "",
        RequestKind::Glossary,
    )
    .await?;
    crate::glossary::candidates(&text, regions)
}

pub struct VisionAttachments {
    parts: Vec<ContentPart>,
}
enum ImageInput<'a> {
    None,
    Page(&'a image::DynamicImage),
    Prepared(&'a VisionAttachments),
}
impl<'a> From<Option<&'a image::DynamicImage>> for ImageInput<'a> {
    fn from(image: Option<&'a image::DynamicImage>) -> Self {
        image.map_or(Self::None, Self::Page)
    }
}
impl ImageInput<'_> {
    fn present(&self) -> bool {
        !matches!(self, Self::None)
    }
}
fn vision_parts(
    image: &image::DynamicImage,
    regions: &[Region],
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Vec<ContentPart>> {
    let mut parts = vec![];
    for r in regions {
        anyhow::ensure!(cancel.is_none_or(|c| !c.is_cancelled()), "Cancelled");
        parts.push(ContentPart::from_text(prompts::region_label(r)?));
        let crop = inference::crop(image, r.bbox);
        // Source-resolution overlapping tiles and PNG bytes are unchanged.
        let stride = 1408u32;
        let columns = crop.width().div_ceil(stride);
        let rows = crop.height().div_ceil(stride);
        anyhow::ensure!(
            columns * rows <= 16,
            "Region is too large for a vision request; split it into smaller regions in Editor"
        );
        for row in 0..rows {
            for column in 0..columns {
                anyhow::ensure!(cancel.is_none_or(|c| !c.is_cancelled()), "Cancelled");
                let x = column * stride;
                let y = row * stride;
                let tile = crop.crop_imm(
                    x,
                    y,
                    1536.min(crop.width() - x),
                    1536.min(crop.height() - y),
                );
                if rows * columns > 1 {
                    parts.push(ContentPart::from_text(format!("Region {} tile row {}/{} column {}/{}. Tiles overlap; read this region once and deduplicate overlapping text.",r.id,row+1,rows,column+1,columns)));
                }
                let mut bytes = std::io::Cursor::new(Vec::new());
                tile.write_to(&mut bytes, image::ImageFormat::Png)?;
                anyhow::ensure!(cancel.is_none_or(|c| !c.is_cancelled()), "Cancelled");
                parts.push(ContentPart::from_binary_base64(
                    "image/png",
                    base64::engine::general_purpose::STANDARD.encode(bytes.into_inner()),
                    None,
                ));
            }
        }
    }
    Ok(parts)
}
/// Takes ownership of the already-loaded image, avoiding another full-page copy.
/// Cancellation waits for the active tile encoder to return before releasing ownership.
pub async fn prepare_vision_region(
    image: image::DynamicImage,
    region: Region,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<VisionAttachments> {
    static SLOTS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)));
    let permit = tokio::select! {
        _ = cancel.cancelled() => bail!("Cancelled"),
        slot = SLOTS.clone().acquire_owned() => slot?,
    };
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        Ok(VisionAttachments {
            parts: vision_parts(&image, &[region], Some(&cancel))?,
        })
    })
    .await?
}
pub async fn transcribe_prepared(
    p: &ProviderProfile,
    key: &str,
    s: &TranslationSettings,
    region: &Region,
    attachments: &VisionAttachments,
) -> Result<String> {
    let text = llm_request(
        p,
        key,
        s,
        std::slice::from_ref(region),
        ImageInput::Prepared(attachments),
        "",
        RequestKind::Transcription,
    )
    .await?;
    Ok(
        validate_response(&text, std::slice::from_ref(region), true)?
            .into_iter()
            .next()
            .context("Missing transcription result")?
            .source,
    )
}

async fn llm_request(
    p: &ProviderProfile,
    key: &str,
    s: &TranslationSettings,
    regions: &[Region],
    image: ImageInput<'_>,
    context: &str,
    kind: RequestKind,
) -> Result<String> {
    let transcription = kind == RequestKind::Transcription;
    let simple = kind == RequestKind::Translation
        && crate::instructions::simple_translation(p, image.present());
    if regions.is_empty() {
        return Ok(if kind == RequestKind::Glossary {
            "NONE"
        } else {
            "{\"regions\":[]}"
        }
        .into());
    }
    p.instructions.validate()?;
    credential_scope(p)?;
    if is_ollama(p) {
        ollama::require_model(&p.endpoint, &p.model, image.present()).await?;
    } else if image.present() && !p.vision {
        bail!("Select an image-capable model or explicitly choose Local OCR")
    };
    let adapter = match p.protocol.as_str() {
        "openai" => AdapterKind::OpenAI,
        "responses" => AdapterKind::OpenAIResp,
        "anthropic" => AdapterKind::Anthropic,
        "gemini" => AdapterKind::Gemini,
        // genai 0.5.3's Ollama adapter uses this same compatibility transport,
        // but drops reasoning_effort. Its OpenAI adapter serializes it, allowing
        // local thinking models to answer before exhausting their context.
        // The explicit resolver below preserves Ollama's endpoint and keyless auth.
        "ollama" => AdapterKind::OpenAI,
        _ => bail!("Unsupported protocol"),
    };
    let url = endpoint(p);
    // The compatibility adapter needs a placeholder; no saved credential is read or sent.
    let key = if is_ollama(p) { "ollama" } else { key }.to_owned();
    let model = p.model.clone();
    let resolver = ServiceTargetResolver::from_resolver_fn(move |_: ServiceTarget| {
        Ok(ServiceTarget {
            endpoint: Endpoint::from_owned(url.clone()),
            auth: AuthData::from_single(key.clone()),
            model: ModelIden::new(adapter, model.clone()),
        })
    });
    let deadline = Duration::from_secs(if is_ollama(p) { 600 } else { 120 });
    let http = crate::http_transport::llm(is_ollama(p))?;
    let client = Client::builder()
        .with_reqwest(http)
        .with_service_target_resolver(resolver)
        .build();
    let schema = (is_ollama(p) && !simple && kind != RequestKind::Glossary).then(|| {
        let mut schema = translation_schema(regions);
        if transcription {
            schema["properties"]["regions"]["minItems"] = json!(regions.len());
            schema["properties"]["regions"]["items"]["properties"]["target"]["const"] = json!("");
        }
        schema
    });
    let prompt = if kind == RequestKind::Glossary {
        crate::instructions::compose(
            &p.instructions,
            s,
            crate::instructions::Task::GlossaryDetection,
            context,
            schema.as_ref(),
        )?
    } else if transcription {
        prompts::transcription(s, schema.as_ref())?
    } else if simple {
        crate::instructions::simple_prompt(&p.instructions, s, context, regions)?
    } else {
        crate::instructions::compose(
            &p.instructions,
            s,
            if image.present() {
                crate::instructions::Task::VisionTranslation
            } else {
                crate::instructions::Task::TextTranslation
            },
            context,
            schema.as_ref(),
        )?
    };
    let mut parts = vec![ContentPart::from_text(prompt.user_preamble)];
    match image {
        ImageInput::Prepared(attachments) => parts.extend(attachments.parts.clone()),
        ImageInput::Page(im) => parts.extend(vision_parts(im, regions, None)?),
        ImageInput::None => {
            if !simple && kind == RequestKind::Translation {
                parts.push(ContentPart::from_text(crate::text_batches::current_text(
                    regions, false,
                )?));
            } else if kind == RequestKind::Glossary {
                parts.push(ContentPart::from_text(prompts::glossary_sources(regions)?));
            }
        }
    }
    let mut request = ChatRequest::new(vec![ChatMessage::user(MessageContent::from_parts(parts))]);
    if !prompt.system.is_empty() {
        request = request.with_system(prompt.system);
    }
    // Ollama uses /v1/chat/completions. JsonSpec becomes
    // response_format: {type: "json_schema", json_schema: {strict: true, ...}};
    // native Ollama's `format` field would be ignored on this transport.
    let policy = if let Some(policy) = &p.thinking_policy {
        policy.clone()
    } else {
        crate::thinking::resolve(p).await
    };
    // Leave optional sampling, context and output limits to the service. Thinking
    // is an explicit user preference; schemas define the response contract.
    let mut options = ChatOptions::default().with_normalize_reasoning_content(true);
    if is_ollama(p) {
        options = options.with_capture_raw_body(true);
    }
    if let Some(effort) = policy.effort(p.thinking) {
        options = options.with_reasoning_effort(effort);
    }
    if let Some(schema) = schema {
        options = options
            .with_capture_raw_body(true)
            .with_response_format(JsonSpec::new("umanga_regions", schema));
    }
    for attempt in 0..3 {
        crate::service_requests::wait(p).await;
        match tokio::time::timeout(
            deadline,
            client.exec_chat(&p.model, request.clone(), Some(&options)),
        )
        .await
        {
            Ok(Ok(response)) => {
                return Ok(final_translation_text(p, &response)?.to_owned());
            }
            Ok(Err(e)) => {
                if matches!(
                    &e,
                    genai::Error::WebModelCall {
                        webc_error: genai::webc::Error::ResponseTooLarge,
                        ..
                    } | genai::Error::WebAdapterCall {
                        webc_error: genai::webc::Error::ResponseTooLarge,
                        ..
                    }
                ) {
                    bail!(
                        "Provider response exceeds the 16 MiB safety limit. Reduce the service output and retry explicitly."
                    );
                }
                if request_timed_out(&e) {
                    bail!("{}", timeout_message(p));
                }
                if is_ollama(p) {
                    if simple || kind == RequestKind::Glossary {
                        bail!(
                            "Ollama request failed. Check the server, model, Thinking setting, and available memory. Retry explicitly; request content is not logged."
                        );
                    }
                    bail!(
                        "Ollama request failed. Check the server, model, Thinking setting, available memory, and structured-output support. Unsupported Thinking choices may be rejected. Retry explicitly; request content is not logged."
                    );
                }
                let delay = retry_delay(&e);
                if attempt < 2
                    && let Some(seconds) = delay
                {
                    tokio::time::sleep(Duration::from_secs(seconds.clamp(1, 60))).await;
                    continue;
                }
                bail!(
                    "Provider request failed; check endpoint, model, Thinking setting, credentials, and quota. Unsupported Thinking choices may be rejected. Request content is not logged."
                )
            }
            Err(_) => {
                bail!("{}", timeout_message(p));
            }
        }
    }
    bail!("Provider retry limit reached")
}

fn lang(s: &str, service: &str) -> String {
    match (service, s) {
        ("baidu", "ja") => "jp",
        ("baidu", "ko") => "kor",
        ("baidu", "fr") => "fra",
        ("baidu", "es") => "spa",
        ("baidu", "ar") => "ara",
        ("baidu", "vi") => "vie",
        ("baidu", "ta") => "tam",
        ("baidu", "zh-Hans") => "zh",
        ("baidu", "zh-Hant") => "cht",
        ("google", "zh-Hans") => "zh-CN",
        ("google", "zh-Hant") => "zh-TW",
        ("deepl", "zh-Hans") => "ZH",
        ("deepl", "zh-Hant") => "ZH-HANT",
        _ => s,
    }
    .into()
}
fn deepl_body(
    text: &[&str],
    source: &str,
    target: &str,
    context: &str,
    glossary_id: Option<&str>,
) -> Value {
    let mut body = json!({"text":text,"source_lang":source.to_uppercase(),"target_lang":target.to_uppercase()});
    if !context.is_empty() {
        body["context"] = json!(context);
    }
    if let Some(id) = glossary_id.filter(|id| !id.is_empty()) {
        body["glossary_id"] = json!(id);
    }
    body
}

pub async fn conventional(
    p: &ProviderProfile,
    key: &str,
    s: &TranslationSettings,
    regions: &[Region],
    context: &str,
) -> Result<Vec<TranslationItem>> {
    if s.mode != "local" {
        bail!("Conventional translation requires Local OCR")
    };
    let client = crate::http_transport::client(crate::http_transport::Transport::Translation)?;
    let source = if p.service == "deepl" && s.source_language.starts_with("zh-") {
        "ZH".into()
    } else {
        lang(&s.source_language, &p.service)
    };
    let target = lang(&s.target_language, &p.service);
    let mut out = Vec::new();
    // Small batches stay below every adapter's documented limits; oversized regions are rejected for review.
    let readable: Vec<_> = regions
        .iter()
        .filter(|r| !r.source.trim().is_empty())
        .collect();
    for batch in readable.chunks(if p.service == "baidu" { 1 } else { 20 }) {
        let text = batch.iter().map(|r| r.source.as_str()).collect::<Vec<_>>();
        if text.iter().any(|t| t.is_empty() || t.len() > 6000) {
            bail!("OCR text is empty or exceeds the service request limit; split the region")
        }
        let request=match p.service.as_str(){
   "google"=>client.post("https://translation.googleapis.com/language/translate/v2").header("X-Goog-Api-Key",key).json(&json!({"q":text,"source":source,"target":target,"format":"text","model":"nmt"})),
   "microsoft"=>client.post("https://api.cognitive.microsofttranslator.com/translate").query(&[("api-version","3.0"),("from",&source),("to",&target)]).header("Ocp-Apim-Subscription-Key",key).header("Ocp-Apim-Subscription-Region",&p.region).json(&text.iter().map(|t|json!({"Text":t})).collect::<Vec<_>>()),
   "deepl"=>{let body=deepl_body(&text,&source,&target,context,s.glossary_enabled.then_some(s.deepl_glossary_id.as_str()));client.post(if key.ends_with(":fx"){"https://api-free.deepl.com/v2/translate"}else{"https://api.deepl.com/v2/translate"}).header("Authorization",format!("DeepL-Auth-Key {key}")).json(&body)},
   "baidu"=>{let salt=crate::types::uid();let q=text.join("\n");let sign=format!("{:x}",md5::compute(format!("{}{}{}{}",p.app_id,q,salt,key)));client.post("https://fanyi-api.baidu.com/api/trans/vip/translate").form(&[("q",q),("from",source.clone()),("to",target.clone()),("appid",p.app_id.clone()),("salt",salt),("sign",sign)])},_=>bail!("Unsupported translation service")};
        crate::service_requests::wait(p).await;
        let mut response = request
            .try_clone()
            .context("Cannot clone service request")?
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Translation service network error; retry explicitly"))?;
        for attempt in 0..2 {
            if response.status().as_u16() != 429 && !response.status().is_server_error() {
                break;
            }
            let delay = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(2u64.pow(attempt + 1))
                .clamp(1, 60);
            tokio::time::sleep(Duration::from_secs(delay)).await;
            crate::service_requests::wait(p).await;
            response = request
                .try_clone()
                .context("Cannot retry service request")?
                .send()
                .await
                .map_err(|_| anyhow::anyhow!("Translation service network error"))?;
        }
        let status = response.status();
        if !status.is_success() {
            bail!(
                "Translation service HTTP {}; credentials and request content omitted",
                status.as_u16()
            )
        };
        let v: Value = serde_json::from_slice(
            &crate::http_transport::bounded_body(response, 16 * 1024 * 1024).await?,
        )?;
        let values: Vec<String> = match p.service.as_str() {
            "google" => v["data"]["translations"]
                .as_array()
                .context("Invalid Google response")?
                .iter()
                .map(|v| v["translatedText"].as_str().unwrap_or("").into())
                .collect(),
            "microsoft" => v
                .as_array()
                .context("Invalid Microsoft response")?
                .iter()
                .map(|v| v["translations"][0]["text"].as_str().unwrap_or("").into())
                .collect(),
            "deepl" => v["translations"]
                .as_array()
                .context("Invalid DeepL response")?
                .iter()
                .map(|v| v["text"].as_str().unwrap_or("").into())
                .collect(),
            _ => {
                let rows = v["trans_result"]
                    .as_array()
                    .context("Baidu rejected the request; check language pair and quota")?;
                vec![
                    rows.iter()
                        .map(|v| v["dst"].as_str().unwrap_or(""))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ]
            }
        };
        if values.len() != batch.len() || values.iter().any(|v| v.trim().is_empty()) {
            bail!("Partial conventional translation response")
        };
        for (r, target) in batch.iter().zip(values) {
            crate::safety::response_text(&r.source, &target)?;
            out.push(TranslationItem {
                id: r.id.clone(),
                source: r.source.clone(),
                target,
                direction: String::new(),
            });
        }
    }
    Ok(out)
}
pub fn cached_catalog(folder: &Path) -> Value {
    std::fs::read(folder.join("catalog.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}))
}
pub async fn refresh_catalog(folder: &Path) -> Result<Value> {
    let c = crate::http_transport::client(crate::http_transport::Transport::Catalog)?;
    let response = c
        .get("https://models.dev/api.json")
        .send()
        .await?
        .error_for_status()?;
    let bytes = crate::http_transport::bounded_body(response, 30_000_000).await?;
    let v: Value = serde_json::from_slice(&bytes)?;
    if !v.is_object() {
        bail!("Invalid catalog")
    };
    let destination = folder.join("catalog.json");
    tokio::task::spawn_blocking(move || store::atomic_write(&destination, &bytes)).await??;
    Ok(v)
}
#[cfg(test)]
mod tests {
    #[test]
    fn deepl_context_is_dialogue_only_and_hosted_glossary_is_independent() {
        let context = crate::prompts::SourceContext {
            pages: vec![crate::prompts::SourceContextPage {
                page_number: 8,
                sources: vec!["先に行こう。".into(), "待って。".into()],
            }],
        };
        let body = super::deepl_body(
            &["こんにちは"],
            "ja",
            "ZH",
            &context.for_service(),
            Some("hosted-glossary"),
        );
        assert_eq!(body["context"], "先に行こう。\n待って。");
        assert_eq!(body["glossary_id"], "hosted-glossary");
        assert_eq!(body["source_lang"], "JA");
        assert!(!body.to_string().contains("Page"));
        let body = super::deepl_body(&["こんにちは"], "ja", "ZH", "", None);
        assert!(body.get("context").is_none());
        assert!(body.get("glossary_id").is_none());
    }
    use super::*;
    #[tokio::test]
    async fn http_deadline_is_recognized_without_logging_request_content() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::path("/slow"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_delay(Duration::from_millis(200)),
            )
            .mount(&server)
            .await;
        let error = reqwest_genai::Client::builder()
            .timeout(Duration::from_millis(25))
            .build()
            .unwrap()
            .get(format!("{}/slow", server.uri()))
            .send()
            .await
            .unwrap_err();
        let wrapped = genai::Error::WebAdapterCall {
            adapter_kind: AdapterKind::OpenAI,
            webc_error: genai::webc::Error::Reqwest(error),
        };
        assert!(request_timed_out(&wrapped));
        assert_eq!(retry_delay(&wrapped), None);
        let profile = ProviderProfile {
            protocol: "ollama".into(),
            ..Default::default()
        };
        assert!(timeout_message(&profile).contains("10 minutes"));
        assert!(timeout_message(&profile).contains("Thinking"));
    }
    #[test]
    fn responses_cannot_supply_coordinates_or_unknown_ids() {
        let r = Region {
            id: "local".into(),
            ..Default::default()
        };
        assert!(
            validate(
                r#"{"regions":[{"id":"cloud","target":"bad"}]}"#,
                std::slice::from_ref(&r)
            )
            .is_err()
        );
        assert!(
            validate(
                r#"{"regions":[{"id":"local","target":"ok"},{"id":"local","target":"again"}]}"#,
                std::slice::from_ref(&r)
            )
            .is_err()
        );
        assert!(
            validate(r#"{"regions":[{"id":"local","target":""}]}"#, &[r])
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn credential_origin_is_bound() {
        let mut p = ProviderProfile::default();
        let a = credential_scope(&p).unwrap();
        p.endpoint = "https://another.example/v1".into();
        assert_ne!(a, credential_scope(&p).unwrap());
    }
}

fn request_timed_out(e: &genai::Error) -> bool {
    match e {
        genai::Error::WebModelCall { webc_error, .. }
        | genai::Error::WebAdapterCall { webc_error, .. } => {
            matches!(webc_error, genai::webc::Error::Reqwest(e) if e.is_timeout())
        }
        _ => false,
    }
}
fn timeout_message(p: &ProviderProfile) -> &'static str {
    if is_ollama(p) {
        "Ollama timed out after 10 minutes. Turn Thinking off where supported or choose a faster model, then retry explicitly."
    } else {
        "Provider timed out; retry explicitly to avoid unintended duplicate billing"
    }
}
fn retry_delay(e: &genai::Error) -> Option<u64> {
    match e {
        genai::Error::WebModelCall { webc_error, .. }
        | genai::Error::WebAdapterCall { webc_error, .. } => match webc_error {
            genai::webc::Error::ResponseFailedStatus {
                status, headers, ..
            } if status.as_u16() == 429 || status.is_server_error() => Some(
                headers
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(2),
            ),
            _ => None,
        },
        genai::Error::HttpError { status, .. }
            if status.as_u16() == 429 || status.is_server_error() =>
        {
            Some(2)
        }
        _ => None,
    }
}

#[cfg(test)]
mod transcription_tests {
    use super::*;
    #[test]
    fn transcription_requires_exact_ids_empty_targets_and_string_sources() {
        let r = Region {
            id: "a".into(),
            ..Default::default()
        };
        for text in [
            r#"{"regions":[{"id":"a","target":""}]}"#,
            r#"{"regions":[{"id":"a","source":""}]}"#,
            r#"{"regions":[]}"#,
            r#"{"regions":[{"id":"wrong","source":"x","target":""}]}"#,
            r#"{"regions":[{"id":"a","source":"x","target":"translated"}]}"#,
            r#"{"regions":[{"id":"a","source":null,"target":""}]}"#,
            r#"{"regions":[{"id":"a","source":"x","target":""},{"id":"a","source":"x","target":""}]}"#,
        ] {
            assert!(validate_response(text, std::slice::from_ref(&r), true).is_err());
        }
        assert_eq!(
            validate_response(
                r#"{"regions":[{"id":"a","source":"","target":""}]}"#,
                &[r],
                true
            )
            .unwrap()
            .len(),
            1
        );
    }
}
