mod api;
mod handler;
mod input;
mod session;
mod ws;

use std::collections::HashMap;
use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::{Router, routing::{delete, get}};
use parking_lot::Mutex;
use tokio::sync::broadcast;
use tower_http::services::ServeDir;
use wew::{
    MultiThreadMessageLoop, MessageLoopAbstract, WindowlessRenderWebView,
    runtime::{LogLevel, RuntimeHandler},
    webview::WebViewAttributes,
};

use crate::api::SessionInfo;
use crate::handler::FrameHandler;
use crate::session::{AppState, Session, SessionCmd};

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

// --- Setup helpers ---

fn resolve_public_dir() -> String {
    if let Ok(dir) = std::env::var("PUBLIC_DIR") {
        return dir;
    }
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

fn ensure_cef_flags() {
    let args: Vec<String> = std::env::args().collect();
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

    let user_args: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();

    use std::os::unix::process::CommandExt;
    let err = Command::new(exe).args(&cef_flags).args(&user_args).exec();
    eprintln!("[setup] Failed to re-exec: {}", err);
    std::process::exit(1);
}

fn setup_tailscale(port: u16) -> bool {
    if std::env::var("NO_TAILSCALE").is_ok() {
        return false;
    }
    match Command::new("tailscale").args(["serve", "--bg", &port.to_string()]).output() {
        Ok(output) if output.status.success() => {
            if let Ok(status_output) = Command::new("tailscale").args(["status", "--json"]).output() {
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
    let _ = Command::new("tailscale").args(["serve", "--https=443", "off"]).output();
}

// --- Main ---

fn main() {
    if wew::is_subprocess() {
        wew::execute_subprocess();
        return;
    }

    ensure_cef_flags();
    let mut _xvfb = ensure_display();

    let width: u32 = 1280;
    let height: u32 = 720;
    let port: u16 = std::env::var("PORT")
        .ok().and_then(|p| p.parse().ok()).unwrap_or(3000);
    let url = std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with("--"))
        .or_else(|| std::env::var("START_URL").ok())
        .unwrap_or_else(|| "https://www.google.com".to_string());
    let cdp_port: Option<u16> = std::env::args()
        .find(|arg| arg.starts_with("--remote-debugging-port="))
        .and_then(|arg| arg.split('=').nth(1).and_then(|v| v.parse().ok()));

    let download_dir = std::env::var("DOWNLOAD_DIR").unwrap_or_else(|_| "./downloads".to_string());
    std::fs::create_dir_all(&download_dir).ok();
    unsafe { std::env::set_var("DOWNLOAD_DIR", &download_dir); }
    eprintln!("[CEF-Browser] Download directory: {}", download_dir);
    eprintln!("[CEF-Browser] Starting with URL: {}", url);
    eprintln!("[CEF-Browser] Viewport: {}x{}", width, height);

    // Create CEF runtime
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

    eprintln!("[CEF-Browser] Waiting for CEF context...");
    ctx_rx.recv().expect("CEF context init failed");
    eprintln!("[CEF-Browser] CEF context ready");

    // Session command channel
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<SessionCmd>();
    let next_id = AtomicU32::new(0);

    let state = Arc::new(AppState {
        sessions: Mutex::new(HashMap::new()),
        cmd_tx: Mutex::new(cmd_tx),
        cdp_port: cdp_port.unwrap_or(9222),
    });

    // Session factory (runs on main thread, owns `runtime`)
    let create_session_fn = |state: &Arc<AppState>, url: &str| -> Arc<Session> {
        let id = next_id.fetch_add(1, Ordering::Relaxed).to_string();
        let (frame_tx, _) = broadcast::channel(8);
        let (event_tx, _) = broadcast::channel(16);
        let current_url = Arc::new(Mutex::new(url.to_string()));
        let current_title = Arc::new(Mutex::new(String::new()));
        let last_frame = Arc::new(Mutex::new(None));

        let handler = FrameHandler {
            frame_tx: frame_tx.clone(),
            event_tx: event_tx.clone(),
            session_id: id.clone(),
            width, height,
            last_hash: std::sync::atomic::AtomicU64::new(0),
            current_url: current_url.clone(),
            current_title: current_title.clone(),
            last_frame: last_frame.clone(),
        };

        let webview = runtime.create_webview(
            url,
            WebViewAttributes {
                width, height,
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
            frame_tx, event_tx,
            webview: Mutex::new(Some(webview)),
            current_url, current_title, last_frame,
        });
        state.sessions.lock().insert(id, session.clone());
        session
    };

    // Initial session
    let initial = create_session_fn(&state, &url);
    eprintln!("[CEF-Browser] Initial session {} ready", initial.id);

    // HTTP/WS server on separate thread
    let state_for_server = state.clone();
    let server_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            let public_dir = resolve_public_dir();
            let app = Router::new()
                .route("/ws", get(ws::ws_handler))
                .route("/api/sessions", get(api::list_sessions).post(api::create_session))
                .route("/api/sessions/{id}", delete(api::delete_session))
                .fallback_service(ServeDir::new(&public_dir))
                .with_state(state_for_server);

            let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
                .await.expect("Failed to bind TCP listener");

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
                .await.expect("Server error");
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
                    cdp_target_id: None, cdp_ws_url: None,
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
    if let Some(ref mut xvfb) = _xvfb {
        let _ = xvfb.kill();
    }
}
