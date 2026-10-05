//! Three-way preference updates. Arrays are atomic; unrelated object fields merge.
use crate::types::AppSettings;
use anyhow::{Result, ensure};
use serde_json::Value;

pub fn merge(base: &AppSettings, next: &AppSettings, current: &AppSettings) -> Result<AppSettings> {
    fn update(base: &Value, next: &Value, current: &Value, path: &str) -> Result<Value> {
        if base == next {
            return Ok(current.clone());
        }
        if let (Some(b), Some(n), Some(c)) =
            (base.as_object(), next.as_object(), current.as_object())
        {
            let mut out = c.clone();
            for (key, value) in n {
                out.insert(
                    key.clone(),
                    update(
                        b.get(key).unwrap_or(&Value::Null),
                        value,
                        c.get(key).unwrap_or(&Value::Null),
                        &format!("{path}.{key}"),
                    )?,
                );
            }
            return Ok(Value::Object(out));
        }
        ensure!(
            current == base || current == next,
            "Settings changed elsewhere: {path}. Reopen Settings before applying."
        );
        Ok(next.clone())
    }
    Ok(serde_json::from_value(update(
        &serde_json::to_value(base)?,
        &serde_json::to_value(next)?,
        &serde_json::to_value(current)?,
        "settings",
    )?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unrelated_changes_merge_and_conflicting_changes_fail() {
        let base = AppSettings::default();
        let mut theme = base.clone();
        theme.appearance.active = "night".into();
        let mut view = base.clone();
        view.library_view = "list".into();
        let saved = merge(&base, &view, &theme).unwrap();
        assert_eq!(saved.library_view, "list");
        assert_eq!(saved.appearance.active, "night");
        let mut a = base.clone();
        a.concurrent_books = 3;
        let mut b = base.clone();
        b.concurrent_books = 4;
        assert!(merge(&base, &b, &a).is_err());
        assert_eq!(merge(&base, &a, &a).unwrap().concurrent_books, 3);
    }
}
