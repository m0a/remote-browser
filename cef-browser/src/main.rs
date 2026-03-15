use std::collections::HashMap;
use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get},
};
use image::{ImageBuffer, Rgb, codecs::jpeg::JpegEncoder};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tower_http::services::ServeDir;
use wew::{
    MultiThreadMessageLoop, MessageLoopAbstract, WindowlessRenderWebView,
    events::{KeyboardEvent, KeyboardEventType, KeyboardModifiers, MouseButton, MouseEvent, Position},
    runtime::{LogLevel, RuntimeHandler},
    webview::{
        Frame, WebViewAttributes, WebViewHandler, WebViewState,
        WindowlessRenderWebViewHandler,
    },
};

// --- Frame encoding ---

fn bgra_to_jpeg(buffer: &[u8], width: u32, height: u32, quality: u8) -> Vec<u8> {
    let pixel_count = (width * height) as usize;
    let mut rgb = Vec::with_capacity(pixel_count * 3);
    for chunk in buffer.chunks_exact(4) {
        rgb.push(chunk[2]); // R (from B)
        rgb.push(chunk[1]); // G
        rgb.push(chunk[0]); // B (from R)
    }

    let img: ImageBuffer<Rgb<u8>, _> =
        ImageBuffer::from_raw(width, height, rgb).expect("invalid image dimensions");
    let mut output = Vec::with_capacity(pixel_count / 4);
    let encoder = JpegEncoder::new_with_quality(&mut output, quality);
    img.write_with_encoder(encoder).expect("JPEG encode failed");
    output
}

// --- CEF Runtime Handler ---

struct RuntimeObserver {
    tx: std::sync::mpsc::Sender<()>,
}

impl RuntimeHandler for RuntimeObserver {
    fn on_context_initialized(&self) {
        eprintln!("[CEF] Context initialized");
        let _ = self.tx.send(());
    }
}

// --- CEF WebView Handler ---

struct FrameHandler {
    frame_tx: broadcast::Sender<FrameData>,
    event_tx: broadcast::Sender<String>,
    session_id: String,
    width: u32,
    height: u32,
    last_hash: std::sync::atomic::AtomicU64,
    current_url: Arc<Mutex<String>>,
    current_title: Arc<Mutex<String>>,
}

#[derive(Clone)]
struct FrameData {
    jpeg: Vec<u8>,
    width: u32,
    height: u32,
}

fn simple_hash(data: &[u8]) -> u64 {
    // Sample ~1024 bytes from the buffer for fast change detection
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
            0 => "alert",
            1 => "confirm",
            2 => "prompt",
            3 => "beforeunload",
            _ => "unknown",
        };
        eprintln!(
            "[CEF] JS dialog suppressed: type={}, message={}, default={}",
            type_name, message_text, default_prompt_text
        );
        let event = serde_json::json!({
            "type": "js_dialog",
            "dialogType": type_name,
            "message": message_text,
            "defaultPrompt": default_prompt_text,
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
            0 => "open",
            1 => "open_multiple",
            2 => "open_folder",
            3 => "save",
            _ => "unknown",
        };
        eprintln!(
            "[CEF] File dialog suppressed: mode={}, title={}, path={}",
            mode_name, title, default_file_path
        );
        let event = serde_json::json!({
            "type": "file_dialog",
            "mode": mode_name,
            "title": title,
            "defaultPath": default_file_path,
        });
        let _ = self.event_tx.send(event.to_string());
    }
}

impl WindowlessRenderWebViewHandler for FrameHandler {
    fn on_frame(&self, frame: &Frame) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
        let count = FRAME_COUNT.fetch_add(1, Ordering::Relaxed);

        // Skip frames with no visual change
        let hash = simple_hash(frame.buffer);
        let prev = self.last_hash.swap(hash, Ordering::Relaxed);
        if prev == hash && count > 0 {
            return;
        }

        if count % 30 == 0 {
            eprintln!(
                "[CEF] Frame #{}: {}x{} buffer={}bytes",
                count,
                frame.width,
                frame.height,
                frame.buffer.len()
            );
        }

        let jpeg = bgra_to_jpeg(frame.buffer, frame.width, frame.height, 60);
        let data = FrameData {
            jpeg,
            width: self.width,
            height: self.height,
        };
        let _ = self.frame_tx.send(data);
    }
}

// --- Input types from WebSocket ---

#[derive(Deserialize)]
#[serde(tag = "type")]
enum InputMessage {
    #[serde(rename = "input_mouse")]
    Mouse {
        #[serde(rename = "eventType")]
        event_type: String,
        x: i32,
        y: i32,
        #[serde(default)]
        button: Option<String>,
        #[serde(default)]
        #[allow(dead_code)]
        buttons: Option<i32>,
        #[serde(rename = "clickCount", default)]
        #[allow(dead_code)]
        click_count: Option<i32>,
    },
    #[serde(rename = "input_touch")]
    Touch {
        #[serde(rename = "eventType")]
        event_type: String,
        #[serde(rename = "touchPoints")]
        touch_points: Vec<TouchPoint>,
    },
    #[serde(rename = "input_scroll")]
    Scroll {
        x: i32,
        y: i32,
        #[serde(rename = "deltaX")]
        delta_x: f64,
        #[serde(rename = "deltaY")]
        delta_y: f64,
    },
    #[serde(rename = "input_key")]
    Key {
        #[serde(rename = "eventType")]
        event_type: String,
        #[allow(dead_code)]
        key: String,
        #[serde(default)]
        #[allow(dead_code)]
        code: Option<String>,
        #[serde(rename = "keyCode", default)]
        key_code: Option<u32>,
        #[serde(default)]
        modifiers: Option<u32>,
        #[serde(default)]
        text: Option<String>,
    },
    #[serde(rename = "input_text")]
    Text { text: String },
    #[serde(rename = "navigate")]
    Navigate { url: String },
    #[serde(rename = "go_back")]
    GoBack {},
    #[serde(rename = "go_forward")]
    GoForward {},
    #[serde(rename = "reload")]
    Reload {},
    #[serde(rename = "webauthn_response")]
    WebAuthnResponse { action: String },
}

#[derive(Deserialize)]
struct TouchPoint {
    x: i32,
    y: i32,
    #[allow(dead_code)]
    id: i32,
    #[allow(dead_code)]
    #[serde(rename = "radiusX", default)]
    radius_x: Option<f64>,
    #[allow(dead_code)]
    #[serde(rename = "radiusY", default)]
    radius_y: Option<f64>,
    #[allow(dead_code)]
    #[serde(default)]
    force: Option<f64>,
}

// --- Session & shared application state ---

struct Session {
    id: String,
    frame_tx: broadcast::Sender<FrameData>,
    event_tx: broadcast::Sender<String>,
    webview: Mutex<Option<wew::webview::WebView<WindowlessRenderWebView>>>,
    current_url: Arc<Mutex<String>>,
    current_title: Arc<Mutex<String>>,
}

// --- Session command channel (main thread handles CEF operations) ---

enum SessionCmd {
    Create {
        url: String,
        reply: tokio::sync::oneshot::Sender<SessionInfo>,
    },
    Delete {
        id: String,
        reply: tokio::sync::oneshot::Sender<bool>,
    },
}

struct AppState {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    cmd_tx: Mutex<std::sync::mpsc::Sender<SessionCmd>>,
    cdp_port: u16,
}

// --- WebSocket handler ---

#[derive(Deserialize)]
struct WsQuery {
    session: Option<String>,
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state, query.session))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AppState>, session_id: Option<String>) {
    let session = {
        let sessions = state.sessions.lock();
        match &session_id {
            Some(id) => sessions.get(id).cloned(),
            None => sessions.values().next().cloned(),
        }
    };
    let Some(session) = session else {
        let _ = socket.send(Message::Text(r#"{"type":"error","message":"Session not found"}"#.into())).await;
        return;
    };

    let mut frame_rx = session.frame_tx.subscribe();
    let mut event_rx = session.event_tx.subscribe();

    loop {
        tokio::select! {
            Ok(frame_data) = frame_rx.recv() => {
                let mut buf = Vec::with_capacity(8 + frame_data.jpeg.len());
                buf.extend_from_slice(&frame_data.width.to_le_bytes());
                buf.extend_from_slice(&frame_data.height.to_le_bytes());
                buf.extend_from_slice(&frame_data.jpeg);
                if socket.send(Message::Binary(buf.into())).await.is_err() {
                    break;
                }
            }
            Ok(event) = event_rx.recv() => {
                if socket.send(Message::Text(event.into())).await.is_err() {
                    break;
                }
            }
            Some(Ok(msg)) = socket.recv() => {
                match msg {
                    Message::Text(text) => {
                        handle_input(&session, &text);
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            else => break,
        }
    }
}

fn handle_input(session: &Session, text: &str) {
    let Ok(input) = serde_json::from_str::<InputMessage>(text) else {
        eprintln!("[WS] Failed to parse input: {}", text);
        return;
    };

    let webview_guard = session.webview.lock();
    let Some(webview) = webview_guard.as_ref() else {
        return;
    };

    match input {
        InputMessage::Mouse {
            event_type,
            x,
            y,
            button,
            ..
        } => {
            let btn = match button.as_deref() {
                Some("left") => MouseButton::Left,
                Some("right") => MouseButton::Right,
                Some("middle") => MouseButton::Middle,
                _ => MouseButton::Left,
            };

            match event_type.as_str() {
                "mousePressed" => {
                    webview.mouse(&MouseEvent::Move(Position { x, y }));
                    webview.mouse(&MouseEvent::Click(btn, true, Some(Position { x, y })));
                }
                "mouseReleased" => {
                    webview.mouse(&MouseEvent::Click(btn, false, Some(Position { x, y })));
                }
                "mouseMoved" => {
                    webview.mouse(&MouseEvent::Move(Position { x, y }));
                }
                _ => {}
            }
        }
        InputMessage::Touch {
            event_type,
            touch_points,
        } => {
            // Convert first touch point to mouse events
            if let Some(tp) = touch_points.first() {
                match event_type.as_str() {
                    "touchStart" => {
                        webview.mouse(&MouseEvent::Move(Position { x: tp.x, y: tp.y }));
                        webview.mouse(&MouseEvent::Click(
                            MouseButton::Left,
                            true,
                            Some(Position { x: tp.x, y: tp.y }),
                        ));
                    }
                    "touchEnd" | "touchCancel" => {
                        webview.mouse(&MouseEvent::Click(
                            MouseButton::Left,
                            false,
                            Some(Position { x: tp.x, y: tp.y }),
                        ));
                    }
                    "touchMove" => {
                        webview.mouse(&MouseEvent::Move(Position { x: tp.x, y: tp.y }));
                    }
                    _ => {}
                }
            }
        }
        InputMessage::Scroll {
            x, y, delta_x, delta_y,
        } => {
            webview.mouse(&MouseEvent::Move(Position { x, y }));
            webview.mouse(&MouseEvent::Wheel(Position {
                x: -(delta_x as i32),
                y: -(delta_y as i32),
            }));
        }
        InputMessage::Key {
            event_type,
            key_code,
            modifiers,
            text,
            ..
        } => {
            let key_code = key_code.unwrap_or(0);
            let mods_raw = modifiers.unwrap_or(0) as u8;
            let mut mods = KeyboardModifiers::empty();
            if mods_raw & 1 != 0 {
                mods |= KeyboardModifiers::Alt;
            }
            if mods_raw & 2 != 0 {
                mods |= KeyboardModifiers::Ctrl;
            }
            // metaKey (4) → Win/Command
            if mods_raw & 4 != 0 {
                mods |= KeyboardModifiers::Win;
            }
            if mods_raw & 8 != 0 {
                mods |= KeyboardModifiers::Shift;
            }

            let ty = match event_type.as_str() {
                "keyDown" | "rawKeyDown" => KeyboardEventType::KeyDown,
                "keyUp" => KeyboardEventType::KeyUp,
                "char" => KeyboardEventType::Char,
                _ => return,
            };

            let char_val = text
                .as_ref()
                .and_then(|t| t.chars().next())
                .map(|c| c as u16)
                .unwrap_or(0);

            webview.keyboard(&KeyboardEvent {
                ty,
                modifiers: mods,
                windows_key_code: key_code,
                native_key_code: 0,
                is_system_key: 0,
                character: char_val,
                unmodified_character: char_val,
                focus_on_editable_field: false,
            });
        }
        InputMessage::Text { text } => {
            // Type each character as keyDown + keyUp
            for ch in text.chars() {
                let char_val = ch as u16;
                webview.keyboard(&KeyboardEvent {
                    ty: KeyboardEventType::Char,
                    modifiers: KeyboardModifiers::empty(),
                    windows_key_code: char_val as u32,
                    native_key_code: 0,
                    is_system_key: 0,
                    character: char_val,
                    unmodified_character: char_val,
                    focus_on_editable_field: true,
                });
            }
        }
        InputMessage::Navigate { url } => {
            eprintln!("[WS] Navigate to: {}", url);
            webview.navigate(&url);
        }
        InputMessage::GoBack {} => {
            webview.go_back();
        }
        InputMessage::GoForward {} => {
            webview.go_forward();
        }
        InputMessage::Reload {} => {
            webview.reload();
        }
        InputMessage::WebAuthnResponse { action } => {
            let msg = serde_json::json!({
                "type": "webauthn_response",
                "action": action,
            });
            eprintln!("[WS] Sending webauthn_response to CEF: {}", msg);
            webview.send_message(&msg.to_string());
        }
    }
}

// --- Session API ---

#[derive(Serialize, Clone)]
struct SessionInfo {
    id: String,
    url: String,
    title: String,
    #[serde(rename = "cdpTargetId", skip_serializing_if = "Option::is_none")]
    cdp_target_id: Option<String>,
    #[serde(rename = "cdpWsUrl", skip_serializing_if = "Option::is_none")]
    cdp_ws_url: Option<String>,
}

/// Fetch CDP targets via raw HTTP to localhost (no external dependencies)
fn fetch_cdp_targets_sync(port: u16) -> Vec<serde_json::Value> {
    use std::io::{BufRead, BufReader, Read, Write};
    let stream = match std::net::TcpStream::connect(format!("127.0.0.1:{}", port)) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).ok();
    let req = format!(
        "GET /json HTTP/1.1\r\nHost: localhost:{}\r\nConnection: close\r\n\r\n",
        port
    );
    let mut writer = stream.try_clone().unwrap();
    if writer.write_all(req.as_bytes()).is_err() { return vec![]; }

    let mut reader = BufReader::new(stream);
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 { break; }
        if line == "\r\n" { break; }
        let lower = line.to_lowercase();
        if let Some(val) = lower.strip_prefix("content-length:") {
            content_length = val.trim().parse().unwrap_or(0);
        }
    }
    if content_length == 0 { return vec![]; }
    let mut body = vec![0u8; content_length];
    if reader.read_exact(&mut body).is_err() { return vec![]; }
    serde_json::from_slice(&body).unwrap_or_default()
}

#[derive(Deserialize)]
struct CreateSessionBody {
    #[serde(default = "default_url")]
    url: String,
}

fn default_url() -> String {
    "https://www.google.com".to_string()
}

async fn list_sessions(State(state): State<Arc<AppState>>) -> Json<Vec<SessionInfo>> {
    let cdp_port = state.cdp_port;
    let cdp_targets: Vec<serde_json::Value> = tokio::task::spawn_blocking(move || {
        fetch_cdp_targets_sync(cdp_port)
    }).await.unwrap_or_default();

    // Only page targets (exclude iframes)
    let page_targets: Vec<&serde_json::Value> = cdp_targets.iter()
        .filter(|t| t["type"].as_str() == Some("page"))
        .collect();

    let sessions = state.sessions.lock();
    let mut used_targets: Vec<bool> = vec![false; page_targets.len()];
    let mut list: Vec<SessionInfo> = sessions.values().map(|s| {
        let url = s.current_url.lock().clone();
        // Match by URL (first unused match)
        let cdp_idx = page_targets.iter().enumerate()
            .position(|(i, t)| !used_targets[i] && t["url"].as_str() == Some(&url));
        if let Some(idx) = cdp_idx { used_targets[idx] = true; }
        let cdp = cdp_idx.map(|i| &page_targets[i]);
        SessionInfo {
            id: s.id.clone(),
            url,
            title: s.current_title.lock().clone(),
            cdp_target_id: cdp.and_then(|t| t["id"].as_str().map(String::from)),
            cdp_ws_url: cdp.and_then(|t| t["webSocketDebuggerUrl"].as_str().map(String::from)),
        }
    }).collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    Json(list)
}

async fn create_session_api(
    State(state): State<Arc<AppState>>,
    body: Option<Json<CreateSessionBody>>,
) -> (StatusCode, Json<SessionInfo>) {
    let url = body.map(|b| b.url.clone()).unwrap_or_else(default_url);
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    {
        let tx = state.cmd_tx.lock().clone();
        let _ = tx.send(SessionCmd::Create { url, reply: reply_tx });
    }
    match reply_rx.await {
        Ok(info) => (StatusCode::CREATED, Json(info)),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(SessionInfo {
            id: String::new(), url: String::new(), title: "Error".to_string(),
            cdp_target_id: None, cdp_ws_url: None,
        })),
    }
}

async fn delete_session_api(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> StatusCode {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    {
        let tx = state.cmd_tx.lock().clone();
        let _ = tx.send(SessionCmd::Delete { id, reply: reply_tx });
    }
    match reply_rx.await {
        Ok(true) => StatusCode::NO_CONTENT,
        _ => StatusCode::NOT_FOUND,
    }
}

// --- Setup helpers ---

/// Resolve PUBLIC_DIR: env var > sibling "public" dir > CWD "public"
fn resolve_public_dir() -> String {
    if let Ok(dir) = std::env::var("PUBLIC_DIR") {
        return dir;
    }
    // Try relative to binary location
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let candidate = exe_dir.join("public");
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
        }
    }
    "public".to_string()
}

/// Ensure DISPLAY is set, spawn Xvfb if needed
fn ensure_display() -> Option<Child> {
    if std::env::var("DISPLAY").is_ok() {
        return None;
    }
    let display = ":99";
    match Command::new("Xvfb")
        .args([display, "-screen", "0", "1280x720x24", "-ac", "-nolisten", "tcp"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => {
            // SAFETY: called before any threads are spawned
            unsafe { std::env::set_var("DISPLAY", display); }
            std::thread::sleep(std::time::Duration::from_secs(1));
            eprintln!("[setup] Xvfb started on {} (PID: {})", display, child.id());
            Some(child)
        }
        Err(e) => {
            eprintln!("[setup] Warning: Failed to start Xvfb: {}", e);
            eprintln!("[setup] Set DISPLAY env var or install xorg-server-xvfb");
            None
        }
    }
}

/// Re-exec with CEF flags if not already present
fn ensure_cef_flags() {
    let args: Vec<String> = std::env::args().collect();

    // If --no-sandbox is already present, flags were injected
    if args.iter().any(|a| a == "--no-sandbox") {
        return;
    }

    let exe = std::env::current_exe().expect("Failed to get current exe path");
    let cdp_port = std::env::var("CDP_PORT").unwrap_or_else(|_| "9222".to_string());

    let cef_flags = [
        "--no-sandbox",
        "--disable-gpu",
        "--disable-gpu-compositing",
        "--disable-software-rasterizer",
        &format!("--remote-debugging-port={}", cdp_port),
        "--lang=ja",
    ];

    // Collect user args (skip argv[0])
    let user_args: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();

    // Re-exec with CEF flags + user args
    use std::os::unix::process::CommandExt;
    let err = Command::new(exe)
        .args(&cef_flags)
        .args(&user_args)
        .exec();

    // exec() only returns on error
    eprintln!("[setup] Failed to re-exec: {}", err);
    std::process::exit(1);
}

/// Setup Tailscale serve (non-blocking, best-effort)
fn setup_tailscale(port: u16) -> bool {
    if std::env::var("NO_TAILSCALE").is_ok() {
        return false;
    }
    match Command::new("tailscale")
        .args(["serve", "--bg", &port.to_string()])
        .output()
    {
        Ok(output) if output.status.success() => {
            // Get hostname for display
            if let Ok(status_output) = Command::new("tailscale")
                .args(["status", "--json"])
                .output()
            {
                if let Ok(status) = serde_json::from_slice::<serde_json::Value>(&status_output.stdout) {
                    if let Some(dns) = status["Self"]["DNSName"].as_str() {
                        let hostname = dns.trim_end_matches('.');
                        eprintln!("TAILSCALE_URL=https://{}/", hostname);
                    }
                }
            }
            true
        }
        _ => {
            eprintln!("[setup] tailscale serve: not available (skipped)");
            false
        }
    }
}

fn teardown_tailscale() {
    let _ = Command::new("tailscale")
        .args(["serve", "--https=443", "off"])
        .output();
}

// --- Main ---

fn main() {
    // CEF subprocess check (must be first)
    if wew::is_subprocess() {
        wew::execute_subprocess();
        return;
    }

    // Ensure CEF flags are present (re-execs if needed)
    ensure_cef_flags();

    // Setup Xvfb if no DISPLAY
    let mut _xvfb = ensure_display();

    let width: u32 = 1280;
    let height: u32 = 720;
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let url = std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with("--"))
        .or_else(|| std::env::var("START_URL").ok())
        .unwrap_or_else(|| "https://www.google.com".to_string());

    // Detect CDP port from command line args (--remote-debugging-port=XXXX)
    let cdp_port: Option<u16> = std::env::args()
        .find(|arg| arg.starts_with("--remote-debugging-port="))
        .and_then(|arg| arg.split('=').nth(1).and_then(|v| v.parse().ok()));

    // Create download directory
    let download_dir = std::env::var("DOWNLOAD_DIR").unwrap_or_else(|_| "./downloads".to_string());
    std::fs::create_dir_all(&download_dir).ok();
    unsafe { std::env::set_var("DOWNLOAD_DIR", &download_dir); }
    eprintln!("[CEF-Browser] Download directory: {}", download_dir);

    eprintln!("[CEF-Browser] Starting with URL: {}", url);
    eprintln!("[CEF-Browser] Viewport: {}x{}", width, height);

    // Create CEF runtime with multi-threaded message loop
    let message_loop = MultiThreadMessageLoop::default();
    let builder = message_loop
        .create_runtime_attributes_builder::<WindowlessRenderWebView>()
        .with_root_cache_path("/tmp/cef-browser-cache")
        .with_cache_path("/tmp/cef-browser-cache")
        .with_log_severity(LogLevel::Error)
        .with_locale("ja")
        .with_accept_language_list("ja,en-US,en");

    let (ctx_tx, ctx_rx) = std::sync::mpsc::channel();
    let runtime = builder
        .build()
        .create_runtime(RuntimeObserver { tx: ctx_tx })
        .expect("Failed to create CEF runtime");

    // Wait for CEF context initialization
    eprintln!("[CEF-Browser] Waiting for CEF context...");
    ctx_rx.recv().expect("CEF context init failed");
    eprintln!("[CEF-Browser] CEF context ready");

    // Session command channel — API handlers send, main thread processes
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<SessionCmd>();
    let next_id = AtomicU32::new(0);

    // Shared state for HTTP server (created early so main thread can also use sessions)
    let state = Arc::new(AppState {
        sessions: Mutex::new(HashMap::new()),
        cmd_tx: Mutex::new(cmd_tx),
        cdp_port: cdp_port.unwrap_or(9222),
    });

    // Helper: create session on main thread (owns `runtime`)
    let create_session_fn = |state: &Arc<AppState>, url: &str| -> Arc<Session> {
        let id = next_id.fetch_add(1, Ordering::Relaxed).to_string();
        let (frame_tx, _) = broadcast::channel(2);
        let (event_tx, _) = broadcast::channel(16);
        let current_url = Arc::new(Mutex::new(url.to_string()));
        let current_title = Arc::new(Mutex::new(String::new()));

        let handler = FrameHandler {
            frame_tx: frame_tx.clone(),
            event_tx: event_tx.clone(),
            session_id: id.clone(),
            width,
            height,
            last_hash: std::sync::atomic::AtomicU64::new(0),
            current_url: current_url.clone(),
            current_title: current_title.clone(),
        };

        let webview = runtime.create_webview(
            url,
            WebViewAttributes {
                width,
                height,
                windowless_frame_rate: 30,
                javascript: true,
                local_storage: true,
                ..Default::default()
            },
            handler,
        ).expect("Failed to create WebView");

        eprintln!("[CEF-Browser] Session {} created: {}", id, url);

        let session = Arc::new(Session {
            id: id.clone(),
            frame_tx,
            event_tx,
            webview: Mutex::new(Some(webview)),
            current_url,
            current_title,
        });
        state.sessions.lock().insert(id, session.clone());
        session
    };

    // Create initial session on main thread
    let initial = create_session_fn(&state, &url);
    eprintln!("[CEF-Browser] Initial session {} ready", initial.id);

    // Start HTTP/WS server on separate thread
    let state_for_server = state.clone();
    let server_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            let public_dir = resolve_public_dir();

            let app = Router::new()
                .route("/ws", get(ws_handler))
                .route("/api/sessions", get(list_sessions).post(create_session_api))
                .route("/api/sessions/{id}", delete(delete_session_api))
                .fallback_service(ServeDir::new(&public_dir))
                .with_state(state_for_server);

            let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
                .await
                .expect("Failed to bind TCP listener");

            eprintln!("[CEF-Browser] HTTP/WS server listening on port {}", port);
            eprintln!("VIEWER_PORT={}", port);
            if let Some(cdp) = cdp_port {
                eprintln!("CDP_PORT={}", cdp);
            }

            let tailscale_enabled = setup_tailscale(port);

            let shutdown = async move {
                tokio::signal::ctrl_c().await.ok();
                eprintln!("\n[CEF-Browser] Shutting down...");
                if tailscale_enabled {
                    teardown_tailscale();
                    eprintln!("[setup] tailscale serve: disabled");
                }
            };

            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
                .expect("Server error");
        });
    });

    // Main thread: process session commands (CEF operations must happen here)
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            SessionCmd::Create { url, reply } => {
                let session = create_session_fn(&state, &url);
                let _ = reply.send(SessionInfo {
                    id: session.id.clone(),
                    url: session.current_url.lock().clone(),
                    title: session.current_title.lock().clone(),
                    cdp_target_id: None,
                    cdp_ws_url: None,
                });
            }
            SessionCmd::Delete { id, reply } => {
                let removed = state.sessions.lock().remove(&id);
                if let Some(session) = removed {
                    let _ = session.event_tx.send(r#"{"type":"session_closed"}"#.to_string());
                    session.webview.lock().take();
                    eprintln!("[CEF-Browser] Session {} closed", id);
                    let _ = reply.send(true);
                } else {
                    let _ = reply.send(false);
                }
            }
        }
    }

    server_thread.join().ok();

    // Cleanup Xvfb
    if let Some(ref mut xvfb) = _xvfb {
        let _ = xvfb.kill();
    }
}
