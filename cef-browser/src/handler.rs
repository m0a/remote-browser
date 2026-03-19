use std::sync::Arc;

use image::{ImageBuffer, Rgb, codecs::jpeg::JpegEncoder};
use parking_lot::Mutex;
use tokio::sync::broadcast;
use wew::webview::{Frame, WebViewHandler, WebViewState, WindowlessRenderWebViewHandler};

#[derive(Clone)]
pub struct FrameData {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub struct FrameHandler {
    pub frame_tx: broadcast::Sender<FrameData>,
    pub event_tx: broadcast::Sender<String>,
    pub session_id: String,
    pub width: u32,
    pub height: u32,
    pub last_hash: std::sync::atomic::AtomicU64,
    pub current_url: Arc<Mutex<String>>,
    pub current_title: Arc<Mutex<String>>,
    pub last_frame: Arc<Mutex<Option<FrameData>>>,
}

fn bgra_to_jpeg(buffer: &[u8], width: u32, height: u32, quality: u8) -> Vec<u8> {
    let pixel_count = (width * height) as usize;
    let mut rgb = Vec::with_capacity(pixel_count * 3);
    for chunk in buffer.chunks_exact(4) {
        rgb.push(chunk[2]);
        rgb.push(chunk[1]);
        rgb.push(chunk[0]);
    }

    let img: ImageBuffer<Rgb<u8>, _> =
        ImageBuffer::from_raw(width, height, rgb).expect("invalid image dimensions");
    let mut output = Vec::with_capacity(pixel_count / 4);
    let encoder = JpegEncoder::new_with_quality(&mut output, quality);
    img.write_with_encoder(encoder).expect("JPEG encode failed");
    output
}

fn simple_hash(data: &[u8]) -> u64 {
    let step = if data.len() > 1024 { data.len() / 1024 } else { 1 };
    let mut hash: u64 = 0xcbf29ce484222325;
    for i in (0..data.len()).step_by(step) {
        hash ^= data[i] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

impl WebViewHandler for FrameHandler {
    fn on_state_change(&self, state: WebViewState) {
        eprintln!("[CEF] WebView state: {:?}", state);
    }

    fn on_title_change(&self, title: &str) {
        eprintln!("[CEF] Title: {}", title);
        *self.current_title.lock() = title.to_string();
        let event = serde_json::json!({ "type": "title", "title": title, "sessionId": self.session_id });
        let _ = self.event_tx.send(event.to_string());
    }

    fn on_url_change(&self, url: &str) {
        eprintln!("[CEF] URL: {}", url);
        *self.current_url.lock() = url.to_string();
        let event = serde_json::json!({ "type": "url", "url": url, "sessionId": self.session_id });
        let _ = self.event_tx.send(event.to_string());
    }

    fn on_js_dialog(&self, dialog_type: u32, message_text: &str, default_prompt_text: &str) {
        let type_name = match dialog_type {
            0 => "alert", 1 => "confirm", 2 => "prompt", 3 => "beforeunload", _ => "unknown",
        };
        eprintln!("[CEF] JS dialog suppressed: type={}, message={}, default={}", type_name, message_text, default_prompt_text);
        let event = serde_json::json!({
            "type": "js_dialog", "dialogType": type_name,
            "message": message_text, "defaultPrompt": default_prompt_text,
        });
        let _ = self.event_tx.send(event.to_string());
    }

    fn on_message(&self, message: &str) {
        eprintln!("[CEF] Page message: {}", message);
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(message) {
            if parsed.get("type").and_then(|t| t.as_str()) == Some("webauthn_request") {
                let _ = self.event_tx.send(message.to_string());
            }
        }
    }

    fn on_download_started(&self, id: u32, url: &str, filename: &str, total_bytes: i64) {
        eprintln!("[CEF] Download started: id={} file={} size={}", id, filename, total_bytes);
        let event = serde_json::json!({
            "type": "download_started",
            "id": id, "url": url, "filename": filename, "totalBytes": total_bytes,
        });
        let _ = self.event_tx.send(event.to_string());
    }

    fn on_download_updated(&self, id: u32, received_bytes: i64, total_bytes: i64, percent_complete: i32, is_complete: bool, is_cancelled: bool) {
        if is_complete || is_cancelled {
            eprintln!("[CEF] Download {}: id={}", if is_complete { "complete" } else { "cancelled" }, id);
        }
        let event = serde_json::json!({
            "type": "download_updated",
            "id": id, "receivedBytes": received_bytes, "totalBytes": total_bytes,
            "percentComplete": percent_complete, "isComplete": is_complete, "isCancelled": is_cancelled,
        });
        let _ = self.event_tx.send(event.to_string());
    }

    fn on_file_dialog(&self, mode: u32, title: &str, default_file_path: &str) {
        let mode_name = match mode {
            0 => "open", 1 => "open_multiple", 2 => "open_folder", 3 => "save", _ => "unknown",
        };
        eprintln!("[CEF] File dialog suppressed: mode={}, title={}, path={}", mode_name, title, default_file_path);
        let event = serde_json::json!({
            "type": "file_dialog", "mode": mode_name, "title": title, "defaultPath": default_file_path,
        });
        let _ = self.event_tx.send(event.to_string());
    }
}

impl WindowlessRenderWebViewHandler for FrameHandler {
    fn on_frame(&self, frame: &Frame) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
        let count = FRAME_COUNT.fetch_add(1, Ordering::Relaxed);

        let hash = simple_hash(frame.buffer);
        let prev = self.last_hash.swap(hash, Ordering::Relaxed);
        if prev == hash && count > 0 {
            return;
        }

        if count % 30 == 0 {
            eprintln!("[CEF] Frame #{}: {}x{} buffer={}bytes", count, frame.width, frame.height, frame.buffer.len());
        }

        let jpeg = bgra_to_jpeg(frame.buffer, frame.width, frame.height, 60);
        let data = FrameData { jpeg, width: self.width, height: self.height };
        *self.last_frame.lock() = Some(data.clone());
        let _ = self.frame_tx.send(data);
    }
}
