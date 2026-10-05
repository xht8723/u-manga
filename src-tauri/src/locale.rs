//! Platform display-language lookup, independent of document language and inference.
pub fn system_locale() -> String {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Globalization::{
            GetUserDefaultLocaleName, GetUserPreferredUILanguages, MUI_LANGUAGE_NAME,
        };
        let (mut count, mut length) = (0, 0);
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            std::ptr::null_mut(),
            &mut length,
        ) != 0
            && length > 0
            && length < 65536
        {
            let mut buffer = vec![0u16; length as usize];
            if GetUserPreferredUILanguages(
                MUI_LANGUAGE_NAME,
                &mut count,
                buffer.as_mut_ptr(),
                &mut length,
            ) != 0
            {
                let end = buffer.iter().position(|v| *v == 0).unwrap_or(buffer.len());
                if end > 0 {
                    return String::from_utf16_lossy(&buffer[..end]);
                }
            }
        }
        let mut buffer = [0u16; 85];
        let n = GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32);
        if n > 1 {
            return String::from_utf16_lossy(&buffer[..n as usize - 1]);
        }
    }
    #[cfg(not(windows))]
    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(name) {
            if !value.trim().is_empty() {
                return value.split('.').next().unwrap_or("en").replace('_', "-");
            }
        }
    }
    "en".into()
}

#[tauri::command]
pub fn system_language() -> String {
    system_locale()
}

pub fn startup_chinese() -> bool {
    let system = system_locale().to_ascii_lowercase().starts_with("zh");
    #[cfg(windows)]
    {
        let root = std::env::var_os("U_MANGA_DATA_DIR")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("APPDATA")
                    .map(|p| std::path::PathBuf::from(p).join("app.umanga.desktop"))
            });
        if let Some(root) = root
            && let Ok(bytes) = std::fs::read(root.join("preferences.json"))
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
        {
            match value.get("uiLanguage").and_then(|v| v.as_str()) {
                Some("en") => return false,
                Some("zh-Hans") => return true,
                _ => (),
            }
        }
    }
    system
}

pub fn startup_text(text: &str) -> String {
    umanga_core::ui_message::UiMessage::from_text(text).render(startup_chinese())
}

#[tauri::command]
pub async fn onboarding_language(
    state: tauri::State<'_, super::AppState>,
    language: umanga_core::types::UiLanguage,
) -> super::Api<()> {
    let _storage = state.storage_change.lock().await;
    let mut next = state.settings.lock().clone();
    next.ui_language = language;
    super::persist_preferences(&state, &next).await?;
    Ok(())
}
