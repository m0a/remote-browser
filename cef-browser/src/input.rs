use serde::Deserialize;
use wew::events::{KeyboardEvent, KeyboardEventType, KeyboardModifiers, MouseButton, MouseEvent, Position};

use crate::session::Session;

#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum InputMessage {
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
pub struct TouchPoint {
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

pub fn handle_input(session: &Session, text: &str) {
    let Ok(input) = serde_json::from_str::<InputMessage>(text) else {
        eprintln!("[WS] Failed to parse input: {}", text);
        return;
    };

    let webview_guard = session.webview.lock();
    let Some(webview) = webview_guard.as_ref() else {
        return;
    };

    match input {
        InputMessage::Mouse { event_type, x, y, button, .. } => {
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
        InputMessage::Touch { event_type, touch_points } => {
            if let Some(tp) = touch_points.first() {
                match event_type.as_str() {
                    "touchStart" => {
                        webview.mouse(&MouseEvent::Move(Position { x: tp.x, y: tp.y }));
                        webview.mouse(&MouseEvent::Click(MouseButton::Left, true, Some(Position { x: tp.x, y: tp.y })));
                    }
                    "touchEnd" | "touchCancel" => {
                        webview.mouse(&MouseEvent::Click(MouseButton::Left, false, Some(Position { x: tp.x, y: tp.y })));
                    }
                    "touchMove" => {
                        webview.mouse(&MouseEvent::Move(Position { x: tp.x, y: tp.y }));
                    }
                    _ => {}
                }
            }
        }
        InputMessage::Scroll { x, y, delta_x, delta_y } => {
            webview.mouse(&MouseEvent::Move(Position { x, y }));
            webview.mouse(&MouseEvent::Wheel(Position {
                x: -(delta_x as i32),
                y: -(delta_y as i32),
            }));
        }
        InputMessage::Key { event_type, key_code, modifiers, text, .. } => {
            let key_code = key_code.unwrap_or(0);
            let mods_raw = modifiers.unwrap_or(0) as u8;
            let mut mods = KeyboardModifiers::empty();
            if mods_raw & 1 != 0 { mods |= KeyboardModifiers::Alt; }
            if mods_raw & 2 != 0 { mods |= KeyboardModifiers::Ctrl; }
            if mods_raw & 4 != 0 { mods |= KeyboardModifiers::Win; }
            if mods_raw & 8 != 0 { mods |= KeyboardModifiers::Shift; }

            let ty = match event_type.as_str() {
                "keyDown" | "rawKeyDown" => KeyboardEventType::KeyDown,
                "keyUp" => KeyboardEventType::KeyUp,
                "char" => KeyboardEventType::Char,
                _ => return,
            };

            let char_val = text.as_ref().and_then(|t| t.chars().next()).map(|c| c as u16).unwrap_or(0);

            webview.keyboard(&KeyboardEvent {
                ty, modifiers: mods, windows_key_code: key_code,
                native_key_code: 0, is_system_key: 0,
                character: char_val, unmodified_character: char_val,
                focus_on_editable_field: false,
            });
        }
        InputMessage::Text { text } => {
            for ch in text.chars() {
                let char_val = ch as u16;
                webview.keyboard(&KeyboardEvent {
                    ty: KeyboardEventType::Char,
                    modifiers: KeyboardModifiers::empty(),
                    windows_key_code: char_val as u32,
                    native_key_code: 0, is_system_key: 0,
                    character: char_val, unmodified_character: char_val,
                    focus_on_editable_field: true,
                });
            }
        }
        InputMessage::Navigate { url } => {
            eprintln!("[WS] Navigate to: {}", url);
            webview.navigate(&url);
        }
        InputMessage::GoBack {} => webview.go_back(),
        InputMessage::GoForward {} => webview.go_forward(),
        InputMessage::Reload {} => webview.reload(),
        InputMessage::WebAuthnResponse { action } => {
            let msg = serde_json::json!({ "type": "webauthn_response", "action": action });
            eprintln!("[WS] Sending webauthn_response to CEF: {}", msg);
            webview.send_message(&msg.to_string());
        }
    }
}
