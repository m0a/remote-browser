use std::sync::Arc;

use axum::{
    extract::{Query, State, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::IntoResponse,
};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::handler::{AudioData, FrameData};
use crate::input::handle_input;
use crate::session::AppState;

#[derive(Deserialize)]
pub struct WsQuery {
    session: Option<String>,
}

/// Encode a FrameData into a binary message with 24-byte header:
/// [width:u32le][height:u32le][dirty_x:u32le][dirty_y:u32le][dirty_w:u32le][dirty_h:u32le][webp...]
fn frame_to_message(f: &FrameData) -> Message {
    let mut buf = Vec::with_capacity(1 + 24 + f.image.len());
    buf.push(0x01u8);  // type tag: video
    buf.extend_from_slice(&f.width.to_le_bytes());
    buf.extend_from_slice(&f.height.to_le_bytes());
    buf.extend_from_slice(&f.dirty_x.to_le_bytes());
    buf.extend_from_slice(&f.dirty_y.to_le_bytes());
    buf.extend_from_slice(&f.dirty_width.to_le_bytes());
    buf.extend_from_slice(&f.dirty_height.to_le_bytes());
    buf.extend_from_slice(&f.image);
    Message::Binary(buf.into())
}

fn audio_to_message(a: &AudioData) -> Message {
    let pcm_bytes = a.pcm.len() * 4;
    let mut buf = Vec::with_capacity(1 + 12 + pcm_bytes);
    buf.push(0x02u8);  // type tag: audio
    buf.extend_from_slice(&(a.sample_rate as u32).to_le_bytes());
    buf.extend_from_slice(&(a.channels as u32).to_le_bytes());
    buf.extend_from_slice(&(a.frames as u32).to_le_bytes());
    for sample in &a.pcm {
        buf.extend_from_slice(&sample.to_le_bytes());
    }
    Message::Binary(buf.into())
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
            let _ = socket.send(frame_to_message(&frame_data)).await;
        }
    }

    let mut frame_rx = session.frame_tx.subscribe();
    let mut event_rx = session.event_tx.subscribe();
    let mut audio_rx = session.audio_tx.subscribe();

    loop {
        tokio::select! {
            result = frame_rx.recv() => {
                match result {
                    Ok(frame_data) => {
                        if socket.send(frame_to_message(&frame_data)).await.is_err() {
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
            result = audio_rx.recv() => {
                match result {
                    Ok(audio_data) => {
                        if socket.send(audio_to_message(&audio_data)).await.is_err() {
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
