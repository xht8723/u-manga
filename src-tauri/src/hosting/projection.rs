//! Browser DTOs contain opaque identities and presentation data, never PC paths/credentials.
use super::Context;
use serde_json::{Value, json};
use umanga_core::{
    library::{Book, BookSummary},
    types::{AppSettings, Page, Project, TranslationSettings},
};
pub fn translation(mut settings: TranslationSettings) -> TranslationSettings {
    settings.glossary.clear();
    settings.deepl_glossary_id.clear();
    settings
}
pub fn bootstrap(settings: &AppSettings) -> Value {
    let mut s = settings.clone();
    s.providers.clear();
    s.library_directory = "hosted-library".into();
    s.translation = translation(s.translation);
    s.setup.completed = true;
    s.reader.zoom = 100;
    json!({"settings":s,"systemLocale":"en","models":[],"catalog":{},"fonts":[],"jobsSmoke":false})
}
fn cover(book: &str, reference: &Option<String>) -> Option<String> {
    reference.as_ref().map(|path| {
        format!(
            "/api/v1/books/{book}/cover?v={}",
            umanga_core::store::digest(path.as_bytes())
        )
    })
}
pub fn summary(mut b: BookSummary) -> BookSummary {
    b.path = format!("book:{}", b.id);
    b.cover = cover(&b.id, &b.cover);
    b
}
pub fn book(mut b: Book) -> Book {
    b.path = format!("book:{}", b.id);
    b.cover = cover(&b.id, &b.cover);
    b.glossary.entries.clear();
    b.glossary.automatic_sources.clear();
    b.glossary.deepl_glossary_id.clear();
    b.overrides.translation = b.overrides.translation.map(translation);
    b.overrides.reader = None; // Browser layout/zoom belongs to that browser.
    b
}
/// Glossary editors explicitly request terms; normal book/reader projections stay compact.
pub fn glossary_book(b: Book) -> Book {
    let glossary = b.glossary.clone();
    let mut result = book(b);
    result.glossary = glossary;
    result
}
pub fn page(mut p: Page, book: &str) -> Page {
    p.source.path.clear();
    p.source.entry = None;
    p.source_stamp.clear();
    p.fingerprint.clear();
    p.rendered = p.rendered.map(|_| {
        format!(
            "/api/v1/books/{book}/pages/{}/image?translated=true&revision={}",
            p.id, p.revision
        )
    });
    p.background = p.background.map(|_| "saved-background".into());
    if let Some(c) = &mut p.cleanup {
        c.path = format!(
            "/api/v1/books/{book}/pages/{}/image?cleaned=true&revision={}",
            p.id, p.revision
        );
        c.key.clear();
    }
    p
}
pub fn project(mut p: Project) -> Project {
    p.path = format!("book:{}", p.id);
    p.settings = translation(p.settings);
    p.pages = p
        .pages
        .into_iter()
        .map(|page| self::page(page, &p.id))
        .collect();
    p
}
pub fn jobs(ctx: &Context, mut value: Value) -> Value {
    let books: std::collections::HashMap<_, _> = ctx
        .books
        .lock()
        .iter()
        .map(|(id, path)| (path.clone(), id.clone()))
        .collect();
    fn redact(value: &mut Value, books: &std::collections::HashMap<String, String>) {
        if let Some(tasks) = value.get_mut("jobs").and_then(Value::as_array_mut) {
            tasks.retain_mut(|j| {
                let path = j["project"].as_str().unwrap_or_default();
                let id = books.get(path).cloned();
                if let Some(id) = id {
                    j["project"] = json!(format!("book:{id}"));
                    true
                } else {
                    false
                }
            });
        }
        if let Some(state) = value.get_mut("state") {
            redact(state, books);
        }
    }
    redact(&mut value, &books);
    value
}
/// Full snapshots are paginated; deltas must keep the individual completed jobs.
pub fn history(value: &mut Value, offset: usize, limit: usize) {
    let Some(tasks) = value.get_mut("jobs").and_then(Value::as_array_mut) else {
        return;
    };
    let mut terminal = Vec::new();
    let mut active = Vec::new();
    for job in std::mem::take(tasks) {
        if is_terminal(&job) {
            terminal.push(job);
        } else {
            active.push(job);
        }
    }
    terminal.sort_by(|a, b| b["created"].as_str().cmp(&a["created"].as_str()));
    let total = terminal.len();
    active.extend(terminal.into_iter().skip(offset).take(limit));
    *tasks = active;
    value["history"] = json!({"offset":offset,"limit":limit,"total":total,"more":offset.saturating_add(limit)<total});
}
pub fn is_terminal(job: &Value) -> bool {
    matches!(
        job["status"].as_str(),
        Some("complete" | "failed" | "cancelled")
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glossary_editor_projection_keeps_terms_and_hides_pc_configuration() {
        let mut glossary = umanga_core::glossary::BookGlossary::default();
        glossary.entries.push(umanga_core::types::GlossaryEntry {
            source: "アリス".into(),
            target: "爱丽丝".into(),
        });
        let b = Book {
            warnings: vec![],
            path: "C:/private/book.umanga".into(),
            id: umanga_core::types::uid(),
            metadata: Default::default(),
            chapters: vec![],
            omitted_page_ids: vec![],
            cover: Some("C:/private/cover.png".into()),
            overrides: umanga_core::library::Overrides {
                translation: Some(TranslationSettings {
                    glossary: glossary.entries.clone(),
                    ..Default::default()
                }),
                reader: Some(Default::default()),
            },
            glossary: glossary.clone(),
            revision: 1,
        };
        assert!(book(b.clone()).glossary.entries.is_empty());
        let projected = glossary_book(b);
        assert_eq!(projected.glossary.entries, glossary.entries);
        assert!(projected.overrides.translation.unwrap().glossary.is_empty());
        assert!(projected.overrides.reader.is_none());
        assert!(projected.path.starts_with("book:"));
        assert!(!projected.cover.unwrap().contains("private"));
    }
    #[test]
    fn bootstrap_never_exposes_profiles_or_library_paths() {
        let mut s = AppSettings {
            library_directory: "C:/private/library".into(),
            ..Default::default()
        };
        s.providers.push(umanga_core::types::ProviderProfile {
            endpoint: "https://private/provider".into(),
            ..Default::default()
        });
        s.translation
            .glossary
            .push(umanga_core::types::GlossaryEntry {
                source: "secret".into(),
                target: "term".into(),
            });
        let json = bootstrap(&s).to_string();
        assert!(!json.contains("C:/private"));
        assert!(!json.contains("private/provider"));
        assert!(!json.contains("secret"));
    }
    #[test]
    fn full_history_is_bounded_without_hiding_active_jobs() {
        let mut jobs: Vec<_> = (0..10_000)
            .map(|id| json!({"id":id,"status":"cancelled","created":format!("{id:05}")}))
            .collect();
        jobs.push(json!({"id":"active","status":"paused"}));
        let mut snapshot = json!({"jobs":jobs,"held":true});
        history(&mut snapshot, 50, 50);
        assert_eq!(snapshot["jobs"].as_array().unwrap().len(), 51);
        assert_eq!(snapshot["jobs"][0]["id"], "active");
        assert_eq!(snapshot["jobs"][1]["id"], 9949);
        assert_eq!(snapshot["history"]["total"], 10_000);
        assert_eq!(snapshot["history"]["more"], true);
        assert_eq!(snapshot["held"], true);
    }
    #[test]
    fn cover_identity_updates_without_exposing_a_path() {
        let first = cover("book", &Some("C:/private/cover-one.png".into())).unwrap();
        let second = cover("book", &Some("C:/private/cover-two.png".into())).unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with("/api/v1/books/book/cover?v="));
        assert!(!first.contains("private"));
        assert!(cover("book", &None).is_none());
    }
}
