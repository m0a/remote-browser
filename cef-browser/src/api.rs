use std::sync::Arc;

use axum::{Json, extract::{Path, State}, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::session::{AppState, SessionCmd};

#[derive(Serialize, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub url: String,
    pub title: String,
    #[serde(rename = "cdpTargetId", skip_serializing_if = "Option::is_none")]
    pub cdp_target_id: Option<String>,
    #[serde(rename = "cdpWsUrl", skip_serializing_if = "Option::is_none")]
    pub cdp_ws_url: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateSessionBody {
    #[serde(default = "default_url")]
    pub url: String,
}

fn default_url() -> String {
    "https://www.google.com".to_string()
}

/// Fetch CDP targets via raw HTTP to localhost
fn fetch_cdp_targets_sync(port: u16) -> Vec<serde_json::Value> {
    use std::io::{BufRead, BufReader, Read, Write};
    let stream = match std::net::TcpStream::connect(format!("127.0.0.1:{}", port)) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).ok();
    let req = format!("GET /json HTTP/1.1\r\nHost: localhost:{}\r\nConnection: close\r\n\r\n", port);
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

pub async fn list_sessions(State(state): State<Arc<AppState>>) -> Json<Vec<SessionInfo>> {
    let cdp_port = state.cdp_port;
    let cdp_targets: Vec<serde_json::Value> = tokio::task::spawn_blocking(move || {
        fetch_cdp_targets_sync(cdp_port)
    }).await.unwrap_or_default();

    let page_targets: Vec<&serde_json::Value> = cdp_targets.iter()
        .filter(|t| t["type"].as_str() == Some("page"))
        .collect();

    let sessions = state.sessions.lock();
    let mut used_targets: Vec<bool> = vec![false; page_targets.len()];
    let mut list: Vec<SessionInfo> = sessions.values().map(|s| {
        let url = s.current_url.lock().clone();
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

pub async fn create_session(
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

pub async fn delete_session(
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
