//! Safe wrappers for tray icon operations that prevent panics from
//! zero-width/height icons reaching muda's NSImage conversion.
//!
//! muda 0.17.1 panics with `ZeroWidth` in `PlatformIcon::to_png()` if an
//! icon with width=0 or height=0 is passed. On macOS this happens inside an
//! `extern "C"` callback (nounwind) and causes an immediate abort.
//!
//! These helpers validate the image before forwarding to Tauri's tray APIs.

use tauri::tray::TrayIcon;
use tauri::{image::Image, path::BaseDirectory, AppHandle, Manager};
use tracing::warn;

/// Validate that a Tauri `Image` has non-zero dimensions.
/// Returns `true` if the image is safe to use as an icon.
fn is_valid_icon(image: &Image<'_>) -> bool {
    image.width() > 0 && image.height() > 0
}

/// Safely set the tray icon, skipping images with zero dimensions.
/// Returns `Ok(())` if the icon was set or skipped (with a warning),
/// `Err` only for unexpected tray errors on valid icons.
pub fn safe_set_icon(tray: &TrayIcon, image: Image<'_>) -> anyhow::Result<()> {
    if !is_valid_icon(&image) {
        warn!(
            "skipping tray icon: invalid dimensions {}x{} (would crash muda)",
            image.width(),
            image.height()
        );
        return Ok(());
    }
    tray.set_icon_with_as_template(Some(image), false)?;
    Ok(())
}

/// Load Dystil's primary tray icon in both packaged and `tauri dev` builds.
/// Tauri's resource resolver points at `target/debug/icons` during development,
/// but icons are not copied there. The manifest directory is the authoritative
/// development fallback; packaged applications continue to use their resources.
pub fn load_main_tray_icon(app: &AppHandle) -> Result<Image<'static>, String> {
    let resource_path = app
        .path()
        .resolve("icons/1024x1024.png", BaseDirectory::Resource)
        .map_err(|error| format!("failed to resolve tray icon resource: {error}"))?;

    match Image::from_path(&resource_path) {
        Ok(image) => Ok(image),
        Err(resource_error) if cfg!(debug_assertions) => {
            let source_path =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/1024x1024.png");
            Image::from_path(&source_path).map_err(|source_error| {
                format!(
                    "failed to load tray icon from resource {} ({resource_error}) or development source {} ({source_error})",
                    resource_path.display(),
                    source_path.display()
                )
            })
        }
        Err(error) => Err(format!(
            "failed to load tray icon from {}: {error}",
            resource_path.display()
        )),
    }
}
