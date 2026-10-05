//! Window geometry is independent of book/preferences formats and reader fullscreen.
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Bounds {
    version: u32,
    monitor: Option<String>,
    width: f64,
    height: f64,
    offset_x: f64,
    offset_y: f64,
}
impl Bounds {
    fn valid(&self) -> bool {
        self.version == 1
            && [self.width, self.height]
                .iter()
                .all(|v| v.is_finite() && (100.0..=32768.0).contains(v))
            && [self.offset_x, self.offset_y]
                .iter()
                .all(|v| v.is_finite() && v.abs() < 1_000_000.0)
    }
}
struct Desktop {
    name: Option<String>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
}
impl From<&tauri::Monitor> for Desktop {
    fn from(m: &tauri::Monitor) -> Self {
        let work = m.work_area();
        Self {
            name: m.name().cloned(),
            x: work.position.x as f64,
            y: work.position.y as f64,
            width: work.size.width as f64,
            height: work.size.height as f64,
            scale: m.scale_factor(),
        }
    }
}
struct Placement {
    size: PhysicalSize<u32>,
    position: PhysicalPosition<i32>,
    minimum: PhysicalSize<u32>,
}
fn placement(saved: Option<&Bounds>, desktop: &Desktop, frame: [f64; 2]) -> Placement {
    let saved = saved.filter(|b| b.valid());
    let fraction = if saved.is_some() { 1.0 } else { 0.9 };
    let max_w = (desktop.width * fraction - frame[0]).max(1.0);
    let max_h = (desktop.height * fraction - frame[1]).max(1.0);
    let width = (saved.map_or(1200.0, |b| b.width) * desktop.scale)
        .min(max_w)
        .max(1.0)
        .floor();
    let height = (saved.map_or(800.0, |b| b.height) * desktop.scale)
        .min(max_h)
        .max(1.0)
        .floor();
    let same_monitor = saved.filter(|b| b.monitor.is_some() && b.monitor == desktop.name);
    let x = same_monitor.map_or((desktop.width - width - frame[0]) / 2.0, |b| {
        b.offset_x * desktop.scale
    });
    let y = same_monitor.map_or((desktop.height - height - frame[1]) / 2.0, |b| {
        b.offset_y * desktop.scale
    });
    Placement {
        size: PhysicalSize::new(width as u32, height as u32),
        position: PhysicalPosition::new(
            (desktop.x + x.clamp(0.0, (desktop.width - width - frame[0]).max(0.0))).round() as i32,
            (desktop.y + y.clamp(0.0, (desktop.height - height - frame[1]).max(0.0))).round()
                as i32,
        ),
        minimum: PhysicalSize::new(
            (960.0 * desktop.scale).min(width) as u32,
            (640.0 * desktop.scale).min(height) as u32,
        ),
    }
}
fn normal(fullscreen: bool, maximized: bool, minimized: bool) -> bool {
    !fullscreen && !maximized && !minimized
}
fn capture(window: &WebviewWindow) -> Option<Bounds> {
    if !normal(
        window.is_fullscreen().ok()?,
        window.is_maximized().ok()?,
        window.is_minimized().ok()?,
    ) {
        return None;
    }
    let monitor = window.current_monitor().ok()??;
    let desktop = Desktop::from(&monitor);
    let size = window.inner_size().ok()?;
    let position = window.outer_position().ok()?;
    let b = Bounds {
        version: 1,
        monitor: desktop.name,
        width: size.width as f64 / desktop.scale,
        height: size.height as f64 / desktop.scale,
        offset_x: (position.x as f64 - desktop.x) / desktop.scale,
        offset_y: (position.y as f64 - desktop.y) / desktop.scale,
    };
    b.valid().then_some(b)
}
#[derive(Default)]
struct Pending {
    generation: u64,
    latest: Option<Bounds>,
}
fn save(path: &std::path::Path, value: &Bounds) {
    if let Ok(bytes) = serde_json::to_vec_pretty(value)
        && let Err(error) = umanga_core::store::atomic_write(path, &bytes)
    {
        eprintln!("Could not save window bounds: {error}");
    }
}
pub fn install(app: &tauri::App, folder: PathBuf) -> anyhow::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    let path = folder.join("window-state.json");
    let saved: Option<Bounds> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(Bounds::valid);
    let monitors = window.available_monitors()?;
    let monitor = saved
        .as_ref()
        .and_then(|b| {
            monitors
                .iter()
                .find(|m| {
                    b.monitor
                        .as_ref()
                        .is_some_and(|name| m.name() == Some(name))
                })
                .cloned()
        })
        .or(window.primary_monitor()?)
        .or(window.current_monitor()?);
    if let Some(monitor) = monitor {
        let desktop = Desktop::from(&monitor);
        // Move while hidden first so Windows applies the destination monitor's DPI.
        window.set_position(PhysicalPosition::new(desktop.x as i32, desktop.y as i32))?;
        let inner = window.inner_size()?;
        let outer = window.outer_size()?;
        let frame = [
            outer.width.saturating_sub(inner.width) as f64,
            outer.height.saturating_sub(inner.height) as f64,
        ];
        let target = placement(saved.as_ref(), &desktop, frame);
        window.set_min_size(Some(target.minimum))?;
        window.set_size(target.size)?;
        window.set_position(target.position)?;
    }
    let pending = Arc::new(Mutex::new(Pending {
        generation: 0,
        latest: capture(&window),
    }));
    let observe = window.clone();
    window.on_window_event(move |event| {
        if !matches!(
            event,
            WindowEvent::Resized(_)
                | WindowEvent::Moved(_)
                | WindowEvent::ScaleFactorChanged { .. }
                | WindowEvent::Focused(true)
                | WindowEvent::CloseRequested { .. }
        ) {
            return;
        }
        let value = capture(&observe);
        let mut state = pending.lock();
        if let Some(value) = value {
            state.latest = Some(value);
        }
        state.generation += 1;
        let generation = state.generation;
        if matches!(event, WindowEvent::CloseRequested { .. }) {
            if let Some(value) = &state.latest {
                save(&path, value);
            }
            return;
        }
        drop(state);
        let pending = pending.clone();
        let path = path.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let _ = tauri::async_runtime::spawn_blocking(move || {
                let state = pending.lock();
                if state.generation == generation
                    && let Some(value) = &state.latest
                {
                    save(&path, value);
                }
            })
            .await;
        });
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn desktop(width: f64, height: f64, scale: f64) -> Desktop {
        Desktop {
            name: Some("display".into()),
            x: -width,
            y: 0.0,
            width,
            height,
            scale,
        }
    }
    fn saved() -> Bounds {
        Bounds {
            version: 1,
            monitor: Some("display".into()),
            width: 1000.0,
            height: 700.0,
            offset_x: 50.0,
            offset_y: 25.0,
        }
    }
    #[test]
    fn compact_defaults_and_whole_frame_fit_across_dpi() {
        for (w, h) in [(1920.0, 1040.0), (3840.0, 2080.0), (1280.0, 680.0)] {
            for scale in [1.0, 1.5, 2.0, 3.0] {
                let d = desktop(w, h, scale);
                let frame = [16.0 * scale, 40.0 * scale];
                let p = placement(None, &d, frame);
                assert!(p.size.width as f64 <= 1200.0 * scale);
                assert!(p.size.height as f64 <= 800.0 * scale);
                assert!(p.size.width as f64 + frame[0] <= w * 0.9 + 1.0);
                assert!(p.size.height as f64 + frame[1] <= h * 0.9 + 1.0);
                assert!(p.minimum.width <= p.size.width && p.minimum.height <= p.size.height);
            }
        }
    }
    #[test]
    fn saved_logical_size_and_monitor_relative_offset_follow_dpi() {
        let p = placement(Some(&saved()), &desktop(3840.0, 2080.0, 2.0), [32.0, 80.0]);
        assert_eq!(p.size, PhysicalSize::new(2000, 1400));
        assert_eq!(p.position, PhysicalPosition::new(-3740, 50));
    }
    #[test]
    fn removed_monitor_centers_and_oversized_bounds_are_clamped() {
        let mut b = saved();
        b.monitor = Some("removed".into());
        let d = desktop(1920.0, 1040.0, 1.0);
        let p = placement(Some(&b), &d, [16.0, 40.0]);
        assert_eq!(p.position, PhysicalPosition::new(-1468, 150));
        b.width = 6000.0;
        b.height = 4000.0;
        b.offset_x = -999.0;
        b.offset_y = 4000.0;
        b.monitor = d.name.clone();
        let p = placement(Some(&b), &d, [16.0, 40.0]);
        assert_eq!(p.size, PhysicalSize::new(1904, 1000));
        assert_eq!(p.position, PhysicalPosition::new(-1920, 0));
    }
    #[test]
    fn invalid_or_unknown_version_defaults_without_migration() {
        for invalid in [
            Bounds {
                version: 99,
                ..saved()
            },
            Bounds {
                width: f64::NAN,
                ..saved()
            },
            Bounds {
                height: 0.0,
                ..saved()
            },
        ] {
            assert!(!invalid.valid());
            assert_eq!(
                placement(Some(&invalid), &desktop(3840.0, 2080.0, 1.0), [16.0, 40.0]).size,
                PhysicalSize::new(1200, 800)
            );
        }
        assert!(serde_json::from_str::<Bounds>("broken").is_err());
    }
    #[test]
    fn fullscreen_maximized_and_minimized_never_capture_normal_bounds() {
        for f in [false, true] {
            for x in [false, true] {
                for m in [false, true] {
                    assert_eq!(normal(f, x, m), !(f || x || m));
                }
            }
        }
    }
    #[test]
    fn window_state_round_trip_is_independent_of_preferences() {
        let b = saved();
        assert_eq!(
            serde_json::from_slice::<Bounds>(&serde_json::to_vec(&b).unwrap()).unwrap(),
            b
        );
    }
}
