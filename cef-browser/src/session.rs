use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::broadcast;
use wew::WindowlessRenderWebView;

use crate::api::SessionInfo;
use crate::handler::{AudioData, FrameData};

pub struct Session {
    pub id: String,
    pub frame_tx: broadcast::Sender<FrameData>,
    pub event_tx: broadcast::Sender<String>,
    pub audio_tx: broadcast::Sender<AudioData>,
    pub webview: Mutex<Option<wew::webview::WebView<WindowlessRenderWebView>>>,
    pub current_url: Arc<Mutex<String>>,
    pub current_title: Arc<Mutex<String>>,
    pub last_frame: Arc<Mutex<Option<FrameData>>>,
}

pub enum SessionCmd {
    Create {
        url: String,
        reply: tokio::sync::oneshot::Sender<SessionInfo>,
    },
    Delete {
        id: String,
        reply: tokio::sync::oneshot::Sender<bool>,
    },
}

pub struct AppState {
    pub sessions: Mutex<HashMap<String, Arc<Session>>>,
    pub cmd_tx: Mutex<std::sync::mpsc::Sender<SessionCmd>>,
    pub cdp_port: u16,
}
