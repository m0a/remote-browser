use std::sync::Arc;

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use image::{ImageBuffer, Rgb, codecs::jpeg::JpegEncoder};
use parking_lot::Mutex;
use serde::Deserialize;
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
    width: u32,
    height: u32,
    last_hash: std::sync::atomic::AtomicU64,
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
        let event = serde_json::json!({ "type": "title", "title": title });
        let _ = self.event_tx.send(event.to_string());
    }

    fn on_url_change(&self, url: &str) {
        eprintln!("[CEF] URL: {}", url);
        let event = serde_json::json!({ "type": "url", "url": url });
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

// --- Shared application state ---

struct AppState {
    frame_tx: broadcast::Sender<FrameData>,
    event_tx: broadcast::Sender<String>,
    webview: Mutex<Option<wew::webview::WebView<WindowlessRenderWebView>>>,
}

// --- WebSocket handler ---

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AppState>) {
    let mut frame_rx = state.frame_tx.subscribe();
    let mut event_rx = state.event_tx.subscribe();

    // Send metadata as first message so client knows dimensions
    // Then send binary JPEG frames with a 8-byte header (width:u32 + height:u32)
    loop {
        tokio::select! {
            // Send frames as binary (header + JPEG)
            Ok(frame_data) = frame_rx.recv() => {
                let mut buf = Vec::with_capacity(8 + frame_data.jpeg.len());
                buf.extend_from_slice(&frame_data.width.to_le_bytes());
                buf.extend_from_slice(&frame_data.height.to_le_bytes());
                buf.extend_from_slice(&frame_data.jpeg);
                if socket.send(Message::Binary(buf.into())).await.is_err() {
                    break;
                }
            }
            // Send events as text JSON
            Ok(event) = event_rx.recv() => {
                if socket.send(Message::Text(event.into())).await.is_err() {
                    break;
                }
            }
            // Receive input from client
            Some(Ok(msg)) = socket.recv() => {
                match msg {
                    Message::Text(text) => {
                        handle_input(&state, &text);
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            else => break,
        }
    }
}

fn handle_input(state: &AppState, text: &str) {
    let Ok(input) = serde_json::from_str::<InputMessage>(text) else {
        eprintln!("[WS] Failed to parse input: {}", text);
        return;
    };

    let webview_guard = state.webview.lock();
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

// --- Main ---

fn main() {
    // CEF subprocess check (must be first)
    if wew::is_subprocess() {
        wew::execute_subprocess();
        return;
    }

    let width: u32 = 1280;
    let height: u32 = 720;
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let url = std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with("--"))
        .unwrap_or_else(|| "https://www.google.com".to_string());

    // Detect CDP port from command line args (--remote-debugging-port=XXXX)
    let cdp_port: Option<u16> = std::env::args()
        .find(|arg| arg.starts_with("--remote-debugging-port="))
        .and_then(|arg| arg.split('=').nth(1).and_then(|v| v.parse().ok()));

    eprintln!("[CEF-Browser] Starting with URL: {}", url);
    eprintln!("[CEF-Browser] Viewport: {}x{}", width, height);

    // Broadcast channels
    let (frame_tx, _) = broadcast::channel::<FrameData>(2);
    let (event_tx, _) = broadcast::channel::<String>(16);

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

    // Create windowless webview
    let handler = FrameHandler {
        frame_tx: frame_tx.clone(),
        event_tx: event_tx.clone(),
        width,
        height,
        last_hash: std::sync::atomic::AtomicU64::new(0),
    };

    let webview = runtime
        .create_webview(
            &url,
            WebViewAttributes {
                width,
                height,
                windowless_frame_rate: 30,
                javascript: true,
                local_storage: true,
                ..Default::default()
            },
            handler,
        )
        .expect("Failed to create WebView");

    eprintln!("[CEF-Browser] WebView created, navigating to {}", url);

    // Shared state
    let state = Arc::new(AppState {
        frame_tx,
        event_tx,
        webview: Mutex::new(Some(webview)),
    });

    // Start HTTP/WS server on tokio runtime
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        let public_dir = std::env::var("PUBLIC_DIR").unwrap_or_else(|_| "public".to_string());

        let app = Router::new()
            .route("/ws", get(ws_handler))
            .fallback_service(ServeDir::new(&public_dir))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
            .await
            .expect("Failed to bind TCP listener");

        eprintln!("[CEF-Browser] HTTP/WS server listening on port {}", port);
        eprintln!("VIEWER_PORT={}", port);
        if let Some(cdp) = cdp_port {
            eprintln!("CDP_PORT={}", cdp);
        }

        axum::serve(listener, app)
            .await
            .expect("Server error");
    });
}
