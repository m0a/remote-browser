use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::broadcast;
use wew::webview::{Frame, WebViewHandler, WebViewState, WindowlessRenderWebViewHandler};

#[derive(Clone)]
pub struct FrameData {
    pub image: Vec<u8>,
    /// Full frame dimensions
    pub width: u32,
    pub height: u32,
    /// Dirty region (the part that changed)
    pub dirty_x: u32,
    pub dirty_y: u32,
    pub dirty_width: u32,
    pub dirty_height: u32,
}

#[derive(Clone)]
pub struct AudioData {
    pub sample_rate: i32,
    pub channels: i32,
    pub frames: i32,
    pub pcm: Vec<f32>,  // interleaved float32
}

pub struct FrameHandler {
    pub frame_tx: broadcast::Sender<FrameData>,
    pub event_tx: broadcast::Sender<String>,
    pub audio_tx: broadcast::Sender<AudioData>,
    pub session_id: String,
    pub current_url: Arc<Mutex<String>>,
    pub current_title: Arc<Mutex<String>>,
    pub last_frame: Arc<Mutex<Option<FrameData>>>,
}

/// Encode a dirty rect region from a full BGRA buffer to WebP
fn bgra_dirty_to_webp(
    buffer: &[u8], buf_width: u32,
    x: u32, y: u32, w: u32, h: u32,
    quality: f32,
) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for row in y..(y + h) {
        let row_start = (row * buf_width + x) as usize * 4;
        for col in 0..w as usize {
            let i = row_start + col * 4;
            rgba.push(buffer[i + 2]); // R
            rgba.push(buffer[i + 1]); // G
            rgba.push(buffer[i]);     // B
            rgba.push(buffer[i + 3]); // A
        }
    }
    let encoder = webp::Encoder::from_rgba(&rgba, w, h);
    encoder.encode(quality).to_vec()
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

    fn on_audio_stream_started(&self, sample_rate: i32, channels: i32) {
        eprintln!("[CEF] Audio stream started: {}Hz {}ch", sample_rate, channels);
        let event = serde_json::json!({
            "type": "audio_started", "sampleRate": sample_rate, "channels": channels,
        });
        let _ = self.event_tx.send(event.to_string());
    }

    fn on_audio_stream_packet(&self, data: &[f32], frames: i32, channels: i32) {
        let _ = self.audio_tx.send(AudioData {
            sample_rate: 48000,
            channels,
            frames,
            pcm: data.to_vec(),
        });
    }

    fn on_audio_stream_stopped(&self) {
        eprintln!("[CEF] Audio stream stopped");
        let event = serde_json::json!({ "type": "audio_stopped" });
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
        let dw = frame.dirty_width;
        let dh = frame.dirty_height;
        if dw == 0 || dh == 0 { return; }

        let is_full = frame.x == 0 && frame.y == 0
            && dw == frame.width && dh == frame.height;

        let image = bgra_dirty_to_webp(
            frame.buffer, frame.width,
            frame.x, frame.y, dw, dh, 40.0,
        );

        let data = FrameData {
            image,
            width: frame.width,
            height: frame.height,
            dirty_x: frame.x,
            dirty_y: frame.y,
            dirty_width: dw,
            dirty_height: dh,
        };

        // Cache full-frame renders for new WS connections
        if is_full {
            *self.last_frame.lock() = Some(data.clone());
        } else {
            // For partial updates, also update the cache with a full render
            // (only if no cached frame exists yet)
            if self.last_frame.lock().is_none() {
                let full_image = bgra_dirty_to_webp(
                    frame.buffer, frame.width,
                    0, 0, frame.width, frame.height, 40.0,
                );
                *self.last_frame.lock() = Some(FrameData {
                    image: full_image,
                    width: frame.width, height: frame.height,
                    dirty_x: 0, dirty_y: 0,
                    dirty_width: frame.width, dirty_height: frame.height,
                });
            }
        }

        let _ = self.frame_tx.send(data);
    }
}
