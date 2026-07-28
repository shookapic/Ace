use std::io::Cursor;

use base64::{engine::general_purpose::STANDARD, Engine};
use tauri::command;

use crate::chat::Attachment;

/// Capture the primary monitor as a PNG and return it as a chat attachment.
/// Ace's own window is excluded from capture (WDA_EXCLUDEFROMCAPTURE on Windows),
/// so it won't appear in its own screenshot.
#[command]
pub fn capture_screen() -> Result<Attachment, String> {
    let monitors = xcap::Monitor::all().map_err(|e| format!("monitor query failed: {e}"))?;
    let monitor = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())
        .ok_or_else(|| "no monitor found".to_string())?;

    let image = monitor
        .capture_image()
        .map_err(|e| format!("screen capture failed: {e}"))?;

    let mut png: Vec<u8> = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("PNG encode failed: {e}"))?;

    Ok(Attachment {
        name: "screenshot.png".to_string(),
        mime: "image/png".to_string(),
        data_base64: STANDARD.encode(&png),
    })
}
