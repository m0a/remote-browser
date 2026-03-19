use std::sync::Arc;

use axum::{
    extract::{Query, State, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::IntoResponse,
};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::input::handle_input;
use crate::session::AppState;

#[derive(Deserialize)]
pub struct WsQuery {
    session: Option<String>,
}

pub async fn ws_handler(
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

    // Send last cached frame immediately so the viewer isn't blank
    {
        let cached = session.last_frame.lock().clone();
        if let Some(frame_data) = cached {
            let mut buf = Vec::with_capacity(8 + frame_data.jpeg.len());
            buf.extend_from_slice(&frame_data.width.to_le_bytes());
            buf.extend_from_slice(&frame_data.height.to_le_bytes());
            buf.extend_from_slice(&frame_data.jpeg);
            let _ = socket.send(Message::Binary(buf.into())).await;
        }
    }

    let mut frame_rx = session.frame_tx.subscribe();
    let mut event_rx = session.event_tx.subscribe();

    loop {
        tokio::select! {
            result = frame_rx.recv() => {
                match result {
                    Ok(frame_data) => {
                        let mut buf = Vec::with_capacity(8 + frame_data.jpeg.len());
                        buf.extend_from_slice(&frame_data.width.to_le_bytes());
                        buf.extend_from_slice(&frame_data.height.to_le_bytes());
                        buf.extend_from_slice(&frame_data.jpeg);
                        if socket.send(Message::Binary(buf.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
            result = event_rx.recv() => {
                match result {
                    Ok(event) => {
                        if socket.send(Message::Text(event.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
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
