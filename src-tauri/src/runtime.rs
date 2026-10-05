use anyhow::Result;
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use umanga_core::store;
include!(concat!(env!("OUT_DIR"), "/embedded.rs"));
pub fn extract(cache: &Path) -> Result<PathBuf> {
    let root = cache.join("native-0.1.0-ort1.24.4-pdf8066");
    for (name, hash, compressed, data) in EMBEDDED {
        let target = root.join(name);
        if target.is_file() && store::file_hash(&target)? == *hash {
            continue;
        }
        let bytes = if *compressed {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(*data).read_to_end(&mut out)?;
            out
        } else {
            data.to_vec()
        };
        if store::digest(&bytes) != *hash {
            anyhow::bail!("Embedded asset failed verification: {name}")
        };
        store::atomic_write(&target, &bytes)?;
    }
    Ok(root)
}

/// Load only verified, absolute application libraries and their system dependencies.
#[cfg(windows)]
pub fn prepare_loader(directory: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::*;
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    unsafe {
        if SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if AddDllDirectory(wide(directory).as_ptr()).is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        for name in [
            "vcruntime140.dll",
            "vcruntime140_1.dll",
            "msvcp140.dll",
            "msvcp140_1.dll",
            "DirectML.dll",
        ] {
            let library = LoadLibraryExW(
                wide(&directory.join(name)).as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            );
            if library.is_null() {
                anyhow::bail!(
                    "Cannot load bundled {name}: {}",
                    std::io::Error::last_os_error()
                );
            }
            // Libraries intentionally remain loaded for the lifetime of inference sessions.
        }
    }
    Ok(())
}
#[cfg(not(windows))]
pub fn prepare_loader(_: &Path) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
pub fn check_webview() -> bool {
    if tauri::webview_version().is_ok() {
        return true;
    }
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::*};
    let wide = |s: &str| {
        crate::locale::startup_text(s)
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    unsafe {
        if MessageBoxW(std::ptr::null_mut(), wide("U-Manga needs Microsoft Edge WebView2. Open Microsoft's runtime setup page? Install the Evergreen runtime, then reopen U-Manga.").as_ptr(), wide("U-Manga · WebView2 setup").as_ptr(), MB_YESNO | MB_ICONINFORMATION) == IDYES {
            ShellExecuteW(std::ptr::null_mut(), wide("open").as_ptr(), wide("https://developer.microsoft.com/microsoft-edge/webview2/#download").as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
        }
    }
    false
}
#[cfg(not(windows))]
pub fn check_webview() -> bool {
    true
}

pub fn startup_error(message: &str) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let text: Vec<_> = crate::locale::startup_text(message)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let title: Vec<_> = crate::locale::startup_text("U-Manga · Startup failed")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
    #[cfg(not(windows))]
    eprintln!("{message}");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_detector_repairs_and_runs_without_an_ocr_directory() {
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../test-output/dot-fix/bundled-detector-{}",
            umanga_core::types::uid()
        ));
        let root = extract(&cache).unwrap();
        let pack = umanga_core::models::catalog()
            .unwrap()
            .into_iter()
            .find(|p| p.id == "rtdetr_int8")
            .unwrap();
        let path = root.join("models").join(&pack.id).join(&pack.files[0].name);
        std::fs::write(&path, b"damaged test copy").unwrap();
        extract(&cache).unwrap();
        assert_eq!(store::file_hash(&path).unwrap(), pack.files[0].sha256);
        std::fs::remove_file(&path).unwrap();
        extract(&cache).unwrap();
        assert!(path.is_file());
        prepare_loader(&root.join("runtime")).unwrap();
        umanga_core::inference::init(&root.join("runtime")).unwrap();
        let mut engine = umanga_core::inference::Inference::with_locations(
            umanga_core::models::ModelLocations {
                bundled: root.join("models"),
                downloaded: PathBuf::new(),
            },
        );
        let sample = image::DynamicImage::new_rgb8(640, 640);
        let start = std::time::Instant::now();
        let result = engine.detect(&sample).unwrap();
        println!(
            "Bundled detector, no download directory: {:?}, {} regions",
            start.elapsed(),
            result.len()
        );
    }
}
