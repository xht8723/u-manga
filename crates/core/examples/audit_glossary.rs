//! Reproducible production validation/snapshot measurement; synthetic data only.
use std::time::Instant;
use umanga_core::{
    glossary::{self, BookGlossary},
    library,
    types::*,
};

fn main() -> anyhow::Result<()> {
    let entries: Vec<_> = (0..10_000)
        .map(|i| GlossaryEntry {
            source: format!("名前{i:05}"),
            target: format!("名称{i:05}"),
        })
        .collect();
    let value = BookGlossary {
        entries: entries.clone(),
        automatic_sources: entries.iter().map(|e| e.source.clone()).collect(),
        ..Default::default()
    };
    let mut durations = Vec::new();
    for n in 0..7 {
        let mut input = value.clone();
        let start = Instant::now();
        glossary::validate(&mut input)?;
        if n >= 2 {
            durations.push(start.elapsed().as_secs_f64() * 1000.);
        }
        assert_eq!(input.entries, value.entries);
        assert_eq!(input.automatic_sources, value.automatic_sources);
    }
    let book = library::Book {
        id: uid(),
        path: String::new(),
        metadata: Default::default(),
        chapters: vec![],
        cover: None,
        omitted_page_ids: vec![],
        overrides: Default::default(),
        glossary: value.clone(),
        revision: 0,
        warnings: vec![],
    };
    let captured = library::effective(&book, &AppSettings::default());
    let bytes = serde_json::to_vec(&captured)?.len();
    println!(
        "{}",
        serde_json::json!({
            "entries":entries.len(),"validationMs":durations,
            "disabledEffectiveSettingsBytes":bytes,"disabledCapturedEntries":captured.glossary.len(),
            "sourceEntriesAfter":book.glossary.entries.len(),
        "debugAssertions":cfg!(debug_assertions),
        "scope":"Production functions; synthetic 10000-entry glossary; 2 warmups and 5 measured validations; no provider or disk reads"
        })
    );
    Ok(())
}
