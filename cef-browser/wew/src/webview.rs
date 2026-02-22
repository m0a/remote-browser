//! This module is used to manage a single web page.
//!
//! ## Communication with Web Pages
//!
//! This library's runtime will inject a global object into web pages for
//! communication between Rust and web pages.
//!
//! ```typescript
//! declare global {
//!     interface Window {
//!         MessageTransport: {
//!             on: (handle: (message: string) => void) => void;
//!             send: (message: string) => void;
//!         };
//!     }
//! }
//! ```
//!
//! Usage example:
//!
//! ```typescript
//! window.MessageTransport.on((message: string) => {
//!     console.log("Received message from Rust:", message);
//! });
//!
//! window.MessageTransport.send("Send message to Rust");
//! ```
//!
//! **`WebViewHandler::on_message`** is used to receive messages sent by
//! **`MessageTransport.send`**, while **`MessageTransport.on`** is used to
//! receive messages sent by **`WebView::send_message`**. Sending and receiving
//! messages are full-duplex and asynchronous.
//!
//! ## WebView Types
//!
//! There are two types of runtime:
//!
//! #### WindowlessRenderWebView
//!
//! Off-screen rendering, also known as OSR mode in CEF. In this mode, the web
//! page does not create a window, and the rendered content is passed to the
//! external through the **`WindowlessRenderWebViewHandler::on_frame`**
//! callback, allowing external code to handle rendering themselves.
//!
//! In this mode, the web page does not automatically handle any mouse,
//! keyboard, or other window-related events because there is no window. You
//! need to manually call the corresponding methods on `WebView` based on events
//! to make it respond to events.
//!
//! #### NativeWindowWebView
//!
//! Window rendering, also known as window mode in CEF. In this mode, the web
//! page creates a window and automatically handles mouse, keyboard, and other
//! window-related events.
//!
//! If you provide a window handle to `WebView`, the `WebView` window will be a
//! child window of that window.
//!
//! If you do not provide a window handle, `WebView` will automatically create a
//! Chromium-style window.

use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    marker::PhantomData,
    mem::MaybeUninit,
    ops::Deref,
    ptr::null,
    sync::Arc,
};

use parking_lot::Mutex;
use raw_window_handle::RawWindowHandle;

use crate::{
    Error, Rect, WindowlessRenderWebView,
    events::{
        IMEAction, KeyboardEvent, KeyboardEventType, KeyboardModifiers, MouseButton, MouseEvent,
    },
    request::{CustomRequestHandlerFactory, ICustomRequestHandlerFactory},
    runtime::{IRuntime, Runtime},
    sys,
    utils::{AnyStringCast, GetSharedRef, ThreadSafePointer},
};

/// Represents the type of cursor
#[repr(u32)]
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum CursorType {
    Pointer = 0,
    Cross = 1,
    Hand = 2,
    IBeam = 3,
    Wait = 4,
    Help = 5,
    EastResize = 6,
    NorthResize = 7,
    NorthEastResize = 8,
    NorthWestResize = 9,
    SouthResize = 10,
    SouthEastResize = 11,
    SouthWestResize = 12,
    WestResize = 13,
    NorthSouthResize = 14,
    EastWestResize = 15,
    NorthEastSouthWestResize = 16,
    NorthWestSouthEastResize = 17,
    ColumnResize = 18,
    RowResize = 19,
    MiddlePanning = 20,
    EastPanning = 21,
    NorthPanning = 22,
    NorthEastPanning = 23,
    NorthWestPanning = 24,
    SouthPanning = 25,
    SouthEastPanning = 26,
    SouthWestPanning = 27,
    WestPanning = 28,
    Move = 29,
    VerticalText = 30,
    Cell = 31,
    ContextMenu = 32,
    Alias = 33,
    Progress = 34,
    NoDrop = 35,
    Copy = 36,
    None = 37,
    NotAllowed = 38,
    ZoomIn = 39,
    ZoomOut = 40,
    Grab = 41,
    Grabbing = 42,
    MiddlePanningVertical = 43,
    MiddlePanningHorizontal = 44,
    Custom = 45,
    DndNone = 46,
    DndMove = 47,
    DndCopy = 48,
    DndLink = 49,
    NumValues = 50,
}

/// Represents the type of a frame
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum FrameType {
    View,
    Popup,
}

/// Represents a rendered frame of a web page
#[derive(Clone, Copy)]
pub struct Frame<'a> {
    pub ty: FrameType,
    /// The buffer of the frame
    pub buffer: &'a [u8],
    /// The x coordinate of the frame
    pub x: u32,
    /// The y coordinate of the frame
    pub y: u32,
    /// The width of the frame
    pub width: u32,
    /// The height of the frame
    pub height: u32,
}

impl std::fmt::Debug for Frame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("ty", &self.ty)
            .field("x", &self.x)
            .field("y", &self.y)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

/// Represents the state of a web page
///
/// The order of events is as follows:
///
/// ```text
/// BeforeLoad -> Loaded -> RequestClose -> Close
///          \ -> LoadError -> Loaded -> RequestClose -> Close
/// ```
///
/// Regardless of whether the loading exists an error, the `Loaded` event is
/// triggered, the difference is that if the loading error occurs, the
/// `LoadError` event is triggered first.
#[repr(u32)]
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum WebViewState {
    /// The web page is before loading
    BeforeLoad = 1,
    /// The web page is loaded
    Loaded = 2,
    /// The web page is loading error
    LoadError = 3,
    /// The web page is requesting to close
    RequestClose = 4,
    /// The web page is closed
    Close = 5,
}

/// WebView handler
///
/// This trait is used to handle web view events.
#[allow(unused)]
pub trait WebViewHandler: Send + Sync {
    /// Called when the cursor changes
    ///
    /// When the web page wants to change the mouse pointer style, it will be
    /// triggered, such as moving to a link.
    fn on_cursor_change(&self, ty: CursorType) {}
    /// Called when the web page state changes
    ///
    /// You need to pay attention to status changes, determine whether loading
    /// was successful, and monitor events related to the page closing.
    fn on_state_change(&self, state: WebViewState) {}

    /// Called when the title changes
    fn on_title_change(&self, title: &str) {}

    /// Called when the fullscreen state changes
    fn on_fullscreen_change(&self, fullscreen: bool) {}

    /// Called when a message is received
    ///
    /// This callback is called when a message is received from the web page.
    fn on_message(&self, message: &str) {}

    /// Called when the URL changes (after navigation completes).
    fn on_url_change(&self, url: &str) {}

    /// Called when a JavaScript dialog is requested (alert/confirm/prompt).
    ///
    /// The native dialog is suppressed and auto-responded.
    /// Override this to implement custom DOM-based dialogs.
    fn on_js_dialog(&self, dialog_type: u32, message_text: &str, default_prompt_text: &str) {}

    /// Called when a file dialog is requested.
    ///
    /// The native dialog is suppressed and cancelled.
    /// Override this to implement custom DOM-based file picker.
    fn on_file_dialog(&self, mode: u32, title: &str, default_file_path: &str) {}
}

/// Windowless render web view handler
///
/// A specific event handler for windowless rendering WebView.
#[allow(unused)]
pub trait WindowlessRenderWebViewHandler: WebViewHandler {
    /// Called when the IME composition rectangle changes
    ///
    /// When the IME region changes, you should notify the external window.
    fn on_ime_rect(&self, rect: Rect) {}

    /// Push a new frame when rendering changes
    ///
    /// This only works in windowless rendering mode.
    ///
    /// #### Note:
    ///
    /// Fixed as BGRA texture buffer, not padded and not aligned.
    ///
    /// It should be noted that if the webview is resized, the width and height
    /// of the texture will also change.
    fn on_frame(&self, frame: &Frame) {}
}

/// WebView configuration attributes
pub struct WebViewAttributes {
    /// Request handler factory.
    pub request_handler_factory: Option<CustomRequestHandlerFactory>,
    /// External native window handle.
    pub window_handle: Option<RawWindowHandle>,
    /// The maximum rate in frames per second (fps).
    pub windowless_frame_rate: u32,
    /// window size width.
    pub width: u32,
    /// window size height.
    pub height: u32,
    /// window device scale factor.
    pub device_scale_factor: f32,
    /// page defalt font size.
    pub default_font_size: u32,
    /// page defalt fixed font size.
    pub default_fixed_font_size: u32,
    /// The minimum font size.
    pub minimum_font_size: u32,
    /// The minimum logical font size.
    pub minimum_logical_font_size: u32,
    /// Controls whether WebGL is enabled.
    pub webgl: bool,
    /// Controls whether databases are enabled.
    pub databases: bool,
    /// Controls whether JavaScript can be executed.
    pub javascript: bool,
    /// Controls whether JavaScript can access the clipboard.
    pub javascript_access_clipboard: bool,
    /// Controls whether JavaScript can be used to close windows that were not
    /// opened via JavaScript.
    pub javascript_close_windows: bool,
    /// Controls whether DOM pasting is supported in the editor via
    /// execCommand("paste").
    pub javascript_dom_paste: bool,
    /// Controls whether local storage can be used.
    pub local_storage: bool,
    /// END values that map to WebPreferences settings.
    pub background_color: u32,
}

unsafe impl Send for WebViewAttributes {}
unsafe impl Sync for WebViewAttributes {}

impl Default for WebViewAttributes {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            window_handle: None,
            device_scale_factor: 1.0,
            windowless_frame_rate: 30,
            default_font_size: 12,
            default_fixed_font_size: 12,
            javascript: true,
            local_storage: true,
            javascript_access_clipboard: false,
            request_handler_factory: None,
            webgl: false,
            databases: false,
            javascript_close_windows: false,
            javascript_dom_paste: false,
            background_color: 0xFFFFFFFF,
            minimum_font_size: 12,
            minimum_logical_font_size: 12,
        }
    }
}

/// WebView configuration attributes builder
#[derive(Default)]
pub struct WebViewAttributesBuilder(WebViewAttributes);

impl WebViewAttributesBuilder {
    /// Set the request handler factory
    ///
    /// This function is used to set the request handler factory.
    pub fn with_request_handler_factory(mut self, value: CustomRequestHandlerFactory) -> Self {
        self.0.request_handler_factory = Some(value);
        self
    }

    /// Set the window handle
    ///
    /// In windowed mode, setting the window handle will set the browser as a
    /// child view.
    ///
    /// In windowless mode, setting the window handle is used to identify
    /// monitor information and as a parent view for dialog boxes, context
    /// menus, and other elements. If not provided, the main screen monitor will
    /// be used, and some features that require a parent view may not work
    /// properly.
    pub fn with_window_handle(mut self, value: RawWindowHandle) -> Self {
        self.0.window_handle = Some(value);
        self
    }

    /// Set the frame rate in windowless rendering mode
    ///
    /// This function is used to set the frame rate in windowless rendering
    /// mode.
    ///
    /// Note that this parameter only works in windowless rendering mode.
    pub fn with_windowless_frame_rate(mut self, value: u32) -> Self {
        self.0.windowless_frame_rate = value;
        self
    }

    /// Set the window width
    ///
    /// This function is used to set the window width.
    ///
    /// Note that this parameter only works in windowless rendering mode.
    pub fn with_width(mut self, value: u32) -> Self {
        self.0.width = value;
        self
    }

    /// Set the window height
    ///
    /// This function is used to set the window height.
    ///
    /// Note that this parameter only works in windowless rendering mode.
    pub fn with_height(mut self, value: u32) -> Self {
        self.0.height = value;
        self
    }

    /// Set the device scale factor
    ///
    /// This function is used to set the device scale factor.
    pub fn with_device_scale_factor(mut self, value: f32) -> Self {
        self.0.device_scale_factor = value;
        self
    }

    /// Set the default font size
    ///
    /// This function is used to set the default font size.
    pub fn with_default_font_size(mut self, value: u32) -> Self {
        self.0.default_font_size = value;
        self
    }

    /// Set the default fixed font size
    ///
    /// This function is used to set the default fixed font size.
    pub fn with_default_fixed_font_size(mut self, value: u32) -> Self {
        self.0.default_fixed_font_size = value;
        self
    }

    /// Set the minimum font size
    ///
    /// This function is used to set the minimum font size.
    pub fn with_minimum_font_size(mut self, value: u32) -> Self {
        self.0.minimum_font_size = value;
        self
    }

    /// Set the minimum logical font size
    ///
    /// This function is used to set the minimum logical font size.
    pub fn with_minimum_logical_font_size(mut self, value: u32) -> Self {
        self.0.minimum_logical_font_size = value;
        self
    }

    /// Set whether local storage is enabled
    ///
    /// This function is used to set whether local storage is enabled.
    pub fn with_local_storage(mut self, value: bool) -> Self {
        self.0.local_storage = value;
        self
    }

    /// Set whether WebGL is enabled
    ///
    /// This function is used to set whether WebGL is enabled.
    pub fn with_webgl(mut self, value: bool) -> Self {
        self.0.webgl = value;
        self
    }

    /// Set whether databases are enabled
    ///
    /// This function is used to set whether databases are enabled.
    pub fn with_databases(mut self, value: bool) -> Self {
        self.0.databases = value;
        self
    }

    /// Set whether JavaScript is enabled
    ///
    /// This function is used to set whether JavaScript is enabled.
    pub fn with_javascript(mut self, value: bool) -> Self {
        self.0.javascript = value;
        self
    }

    /// Set whether JavaScript can access the clipboard
    ///
    /// This function is used to set whether JavaScript can access the
    /// clipboard.
    pub fn with_javascript_access_clipboard(mut self, value: bool) -> Self {
        self.0.javascript_access_clipboard = value;
        self
    }

    /// Set whether JavaScript can be used to close windows that were not opened
    /// via JavaScript.
    ///
    /// This function is used to set whether JavaScript can be used to close
    /// windows that were not opened via JavaScript.
    pub fn with_javascript_close_windows(mut self, value: bool) -> Self {
        self.0.javascript_close_windows = value;
        self
    }

    /// Set whether DOM pasting is supported in the editor via
    /// execCommand("paste").
    ///
    /// This function is used to set whether DOM pasting is supported in the
    /// editor via execCommand("paste").
    pub fn with_javascript_dom_paste(mut self, value: bool) -> Self {
        self.0.javascript_dom_paste = value;
        self
    }

    /// Set the background color
    ///
    /// This function is used to set the background color.
    pub fn with_background_color(mut self, value: u32) -> Self {
        self.0.background_color = value;
        self
    }

    pub fn build(self) -> WebViewAttributes {
        self.0
    }
}

impl Deref for WebViewAttributesBuilder {
    type Target = WebViewAttributes;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(crate) struct IWebView {
    mouse_event: Mutex<sys::MouseEvent>,
    // The runtime may use a custom request interceptor; a reference is kept here to ensure correct
    // lifetime management.
    #[allow(unused)]
    request_handler_factory: Option<Arc<ICustomRequestHandlerFactory>>,
    context: ThreadSafePointer<WebViewContext>,
    raw: Mutex<ThreadSafePointer<c_void>>,
}

impl IWebView {
    fn new<R, W>(
        runtime: &Runtime<R, W>,
        url: &str,
        attr: WebViewAttributes,
        handler: MixWebviewHnadler,
    ) -> Result<Self, Error> {
        let runtime = runtime.get_shared_ref();
        let raw_runtime = runtime.get_raw();

        let options = sys::WebViewSettings {
            width: attr.width,
            height: attr.height,
            webgl: attr.webgl,
            databases: attr.databases,
            local_storage: attr.local_storage,
            background_color: attr.background_color,
            javascript: attr.javascript,
            javascript_access_clipboard: attr.javascript_access_clipboard,
            javascript_close_windows: attr.javascript_close_windows,
            javascript_dom_paste: attr.javascript_dom_paste,
            minimum_font_size: attr.minimum_font_size as _,
            minimum_logical_font_size: attr.minimum_logical_font_size as _,
            device_scale_factor: attr.device_scale_factor,
            windowless_frame_rate: attr.windowless_frame_rate,
            default_fixed_font_size: attr.default_fixed_font_size as _,
            default_font_size: attr.default_font_size as _,
            window_handle: {
                #[cfg(not(target_os = "linux"))]
                let mut value = null();

                #[cfg(target_os = "linux")]
                let mut value = 0;

                if let Some(it) = &attr.window_handle {
                    value = match it {
                        #[cfg(target_os = "linux")]
                        RawWindowHandle::Xlib(it) => it.window,
                        #[cfg(target_os = "windows")]
                        RawWindowHandle::Win32(it) => it.hwnd.get() as _,
                        #[cfg(target_os = "macos")]
                        RawWindowHandle::AppKit(it) => it.ns_view.as_ptr() as _,
                        _ => unimplemented!("Unsupported window handle type: {:?}", it),
                    };
                }

                value
            },
            request_handler_factory: if let Some(it) = &attr.request_handler_factory {
                it.as_raw().as_ptr() as _
            } else {
                null()
            },
        };

        let context: *mut WebViewContext = Box::into_raw(Box::new(WebViewContext {
            runtime: Some(runtime),
            handler,
        }));

        let url = CString::new(url).unwrap();
        let ptr = unsafe {
            sys::create_webview(
                raw_runtime.as_ptr(),
                url.as_raw(),
                &options,
                sys::WebViewHandler {
                    on_cursor: Some(on_cursor_callback),
                    on_state_change: Some(on_state_change_callback),
                    on_ime_rect: Some(on_ime_rect_callback),
                    on_frame: Some(on_frame_callback),
                    on_title_change: Some(on_title_change_callback),
                    on_fullscreen_change: Some(on_fullscreen_change_callback),
                    on_message: Some(on_message_callback),
                    on_url_change: Some(on_url_change_callback),
                    on_js_dialog: Some(on_js_dialog_callback),
                    on_file_dialog: Some(on_file_dialog_callback),
                    context: context as _,
                },
            )
        };

        let raw = if ptr.is_null() {
            return Err(Error::FailedToCreateWebView);
        } else {
            ThreadSafePointer::new(ptr)
        };

        Ok(Self {
            raw: Mutex::new(raw),
            context: ThreadSafePointer::new(context),
            mouse_event: Mutex::new(unsafe { std::mem::zeroed() }),
            request_handler_factory: attr
                .request_handler_factory
                .as_ref()
                .map(|it| it.get_shared_ref()),
        })
    }
}

impl Drop for IWebView {
    fn drop(&mut self) {
        unsafe {
            sys::close_webview(self.raw.lock().as_ptr());
        }

        drop(unsafe { Box::from_raw(self.context.as_ptr()) });
    }
}

/// Represents an opened web page
#[allow(unused)]
pub struct WebView<W> {
    _w: PhantomData<W>,
    inner: Arc<IWebView>,
}

impl<W> GetSharedRef for WebView<W> {
    type Ref = Arc<IWebView>;

    fn get_shared_ref(&self) -> Self::Ref {
        self.inner.clone()
    }
}

impl<W> WebView<W> {
    pub(crate) fn new<R>(
        runtime: &Runtime<R, W>,
        url: &str,
        attr: WebViewAttributes,
        handler: MixWebviewHnadler,
    ) -> Result<Self, Error> {
        Ok(Self {
            _w: PhantomData,
            inner: Arc::new(IWebView::new(runtime, url, attr, handler)?),
        })
    }

    /// Get the window handle
    ///
    /// This function is used to get the window handle.
    pub fn window_handle(&self) -> Option<RawWindowHandle> {
        let handle = unsafe { sys::webview_get_window_handle(self.inner.raw.lock().as_ptr()) };

        let mut value = MaybeUninit::<RawWindowHandle>::uninit();

        #[cfg(target_os = "linux")]
        if handle == 0 {
            return None;
        } else {
            unsafe {
                value.as_mut_ptr().write(RawWindowHandle::Xlib(
                    raw_window_handle::XlibWindowHandle::new(handle),
                ));
            }
        }

        #[cfg(not(target_os = "linux"))]
        if handle.is_null() {
            return None;
        } else {
            #[cfg(target_os = "windows")]
            unsafe {
                value.as_mut_ptr().write(RawWindowHandle::Win32(
                    raw_window_handle::Win32WindowHandle::new(
                        std::num::NonZeroIsize::new(handle as _).unwrap(),
                    ),
                ));
            }

            #[cfg(target_os = "macos")]
            unsafe {
                value.as_mut_ptr().write(RawWindowHandle::AppKit(
                    raw_window_handle::AppKitWindowHandle::new(std::ptr::NonNull::new_unchecked(
                        handle as _,
                    )),
                ));
            }
        }

        Some(unsafe { value.assume_init() })
    }

    /// Send a message
    ///
    /// This function is used to send a message to the web page.
    ///
    /// Messages sent from the web page are received through the
    /// **`WebViewHandler::on_message`** callback.
    pub fn send_message(&self, message: &str) {
        let message = CString::new(message).unwrap();

        unsafe {
            sys::webview_send_message(self.inner.raw.lock().as_ptr(), message.as_raw());
        }
    }

    /// Set whether developer tools are enabled
    ///
    /// This function is used to set whether developer tools are enabled.
    pub fn devtools_enabled(&self, enable: bool) {
        unsafe { sys::webview_set_devtools_state(self.inner.raw.lock().as_ptr(), enable) }
    }

    /// Navigate to a URL
    pub fn navigate(&self, url: &str) {
        let url = CString::new(url).unwrap();
        unsafe { sys::webview_navigate(self.inner.raw.lock().as_ptr(), url.as_raw()) }
    }

    /// Go back in history
    pub fn go_back(&self) {
        unsafe { sys::webview_go_back(self.inner.raw.lock().as_ptr()) }
    }

    /// Go forward in history
    pub fn go_forward(&self) {
        unsafe { sys::webview_go_forward(self.inner.raw.lock().as_ptr()) }
    }

    /// Reload the page
    pub fn reload(&self) {
        unsafe { sys::webview_reload(self.inner.raw.lock().as_ptr()) }
    }
}

impl WebView<WindowlessRenderWebView> {
    /// Send a mouse event
    ///
    /// This function is used to send mouse events.
    ///
    /// Note that this function only works in windowless rendering mode.
    pub fn mouse(&self, action: &MouseEvent) {
        let mut event = self.inner.mouse_event.lock();

        match action {
            MouseEvent::Move(pos) => unsafe {
                event.x = pos.x;
                event.y = pos.y;

                sys::webview_mouse_move(self.inner.raw.lock().as_ptr(), *event)
            },
            MouseEvent::Wheel(pos) => unsafe {
                sys::webview_mouse_wheel(self.inner.raw.lock().as_ptr(), *event, pos.x, pos.y)
            },
            MouseEvent::Click(button, is_pressed, pos) => {
                if let Some(pos) = pos {
                    event.x = pos.x;
                    event.y = pos.y;
                }

                if *is_pressed {
                    event.modifiers |= match button {
                        MouseButton::Left => sys::EventFlags::WEW_EVENTFLAG_LEFT_MOUSE_BUTTON,
                        MouseButton::Right => sys::EventFlags::WEW_EVENTFLAG_RIGHT_MOUSE_BUTTON,
                        MouseButton::Middle => sys::EventFlags::WEW_EVENTFLAG_MIDDLE_MOUSE_BUTTON,
                    } as u32;
                } else {
                    event.modifiers = 0;
                };

                unsafe {
                    sys::webview_mouse_click(
                        self.inner.raw.lock().as_ptr(),
                        *event,
                        (*button).into(),
                        *is_pressed,
                    )
                }
            }
        }
    }

    /// Send a keyboard event
    ///
    /// This function is used to send keyboard events.
    ///
    /// Note that this function only works in windowless rendering mode.
    pub fn keyboard(&self, event: &KeyboardEvent) {
        let mut modifiers = sys::EventFlags::WEW_EVENTFLAG_NONE as u32;
        for it in KeyboardModifiers::all() {
            if event.modifiers.contains(it) {
                let flag: sys::EventFlags = it.into();
                modifiers |= flag as u32;
            }
        }

        unsafe {
            sys::webview_keyboard(
                self.inner.raw.lock().as_ptr(),
                sys::KeyEvent {
                    modifiers,
                    character: event.character,
                    unmodified_character: event.unmodified_character,
                    windows_key_code: event.windows_key_code as i32,
                    native_key_code: event.native_key_code as i32,
                    is_system_key: event.is_system_key as i32,
                    focus_on_editable_field: event.focus_on_editable_field as i32,
                    type_: event.ty.into(),
                },
            )
        }
    }

    /// Send an IME event
    ///
    /// This function is used to send IME events.
    ///
    /// Note that this function only works in windowless rendering mode.
    pub fn ime(&self, action: &IMEAction) {
        let input = match action {
            IMEAction::Composition(it) | IMEAction::Pre(it, _, _) => CString::new(*it).unwrap(),
        };

        match action {
            IMEAction::Composition(_) => unsafe {
                sys::webview_ime_composition(self.inner.raw.lock().as_ptr(), input.as_raw())
            },
            IMEAction::Pre(_, x, y) => unsafe {
                sys::webview_ime_set_composition(
                    self.inner.raw.lock().as_ptr(),
                    input.as_raw(),
                    *x,
                    *y,
                )
            },
        }
    }

    /// Resize the window
    ///
    /// This function is used to resize the window.
    ///
    /// Note that this function only works in windowless rendering mode.
    pub fn resize(&self, width: u32, height: u32) {
        unsafe {
            sys::webview_resize(
                self.inner.raw.lock().as_ptr(),
                width as c_int,
                height as c_int,
            )
        }
    }

    /// Set the focus state
    ///
    /// This function is used to set the focus state.
    ///
    /// Note that this function only works in windowless rendering mode.
    pub fn focus(&self, state: bool) {
        unsafe { sys::webview_set_focus(self.inner.raw.lock().as_ptr(), state) }
    }
}

impl From<sys::WebViewState> for WebViewState {
    fn from(value: sys::WebViewState) -> Self {
        match value {
            sys::WebViewState::WEW_BEFORE_LOAD => Self::BeforeLoad,
            sys::WebViewState::WEW_LOADED => Self::Loaded,
            sys::WebViewState::WEW_LOAD_ERROR => Self::LoadError,
            sys::WebViewState::WEW_REQUEST_CLOSE => Self::RequestClose,
            sys::WebViewState::WEW_CLOSE => Self::Close,
        }
    }
}

impl From<KeyboardEventType> for sys::KeyEventType {
    fn from(val: KeyboardEventType) -> Self {
        match val {
            KeyboardEventType::KeyDown => sys::KeyEventType::WEW_KEYEVENT_KEYDOWN,
            KeyboardEventType::KeyUp => sys::KeyEventType::WEW_KEYEVENT_KEYUP,
            KeyboardEventType::Char => sys::KeyEventType::WEW_KEYEVENT_CHAR,
        }
    }
}

impl From<KeyboardModifiers> for sys::EventFlags {
    fn from(val: KeyboardModifiers) -> Self {
        match val {
            KeyboardModifiers::None => sys::EventFlags::WEW_EVENTFLAG_NONE,
            KeyboardModifiers::Win => sys::EventFlags::WEW_EVENTFLAG_COMMAND_DOWN,
            KeyboardModifiers::Shift => sys::EventFlags::WEW_EVENTFLAG_SHIFT_DOWN,
            KeyboardModifiers::Ctrl => sys::EventFlags::WEW_EVENTFLAG_CONTROL_DOWN,
            KeyboardModifiers::Alt => sys::EventFlags::WEW_EVENTFLAG_ALT_DOWN,
            KeyboardModifiers::Command => sys::EventFlags::WEW_EVENTFLAG_COMMAND_DOWN,
            KeyboardModifiers::CapsLock => sys::EventFlags::WEW_EVENTFLAG_CAPS_LOCK_ON,
            _ => sys::EventFlags::WEW_EVENTFLAG_NONE,
        }
    }
}

impl From<MouseButton> for sys::MouseButton {
    fn from(val: MouseButton) -> Self {
        match val {
            MouseButton::Left => sys::MouseButton::WEW_MBT_LEFT,
            MouseButton::Middle => sys::MouseButton::WEW_MBT_MIDDLE,
            MouseButton::Right => sys::MouseButton::WEW_MBT_RIGHT,
        }
    }
}

struct WebViewContext {
    runtime: Option<Arc<IRuntime>>,
    handler: MixWebviewHnadler,
}

pub(crate) enum MixWebviewHnadler {
    WebViewHandler(Box<dyn WebViewHandler>),
    WindowlessRenderWebViewHandler(Box<dyn WindowlessRenderWebViewHandler>),
}

extern "C" fn on_state_change_callback(state: sys::WebViewState, context: *mut c_void) {
    if context.is_null() {
        return;
    }

    let state = WebViewState::from(state);
    let context = unsafe { &mut *(context as *mut WebViewContext) };

    // Only after all webviews are closed can the runtime be closed. Here, we clear
    // the reference held by the current webview.
    //
    // If all webviews are closed, the runtime reference will be cleared,
    // and only then will the runtime's Drop be triggered.
    if state == WebViewState::Close {
        drop(context.runtime.take());
    }

    match &context.handler {
        MixWebviewHnadler::WebViewHandler(handler) => handler.on_state_change(state),
        MixWebviewHnadler::WindowlessRenderWebViewHandler(handler) => {
            handler.on_state_change(state)
        }
    }
}

extern "C" fn on_ime_rect_callback(rect: sys::Rect, context: *mut c_void) {
    if context.is_null() {
        return;
    }

    let context = unsafe { &*(context as *mut WebViewContext) };

    if let MixWebviewHnadler::WindowlessRenderWebViewHandler(handler) = &context.handler {
        handler.on_ime_rect(Rect {
            x: rect.x as u32,
            y: rect.y as u32,
            width: rect.width as u32,
            height: rect.height as u32,
        })
    }
}

extern "C" fn on_frame_callback(frame: *const sys::Frame, context: *mut c_void) {
    if context.is_null() || frame.is_null() {
        return;
    }

    let raw_frame = unsafe { &*frame };
    let context = unsafe { &*(context as *mut WebViewContext) };

    let frame = Frame {
        x: raw_frame.x,
        y: raw_frame.y,
        width: raw_frame.width,
        height: raw_frame.height,
        buffer: unsafe {
            std::slice::from_raw_parts(
                raw_frame.buffer as *const u8,
                raw_frame.width as usize * raw_frame.height as usize * 4,
            )
        },
        ty: if raw_frame.is_popup {
            FrameType::Popup
        } else {
            FrameType::View
        },
    };

    if let MixWebviewHnadler::WindowlessRenderWebViewHandler(handler) = &context.handler {
        handler.on_frame(&frame);
    }
}

extern "C" fn on_title_change_callback(title: *const c_char, context: *mut c_void) {
    if context.is_null() || title.is_null() {
        return;
    }

    let context = unsafe { &*(context as *mut WebViewContext) };

    if let Ok(title) = unsafe { CStr::from_ptr(title) }.to_str() {
        match &context.handler {
            MixWebviewHnadler::WebViewHandler(handler) => handler.on_title_change(title),
            MixWebviewHnadler::WindowlessRenderWebViewHandler(handler) => {
                handler.on_title_change(title)
            }
        }
    }
}
extern "C" fn on_fullscreen_change_callback(fullscreen: bool, context: *mut c_void) {
    if context.is_null() {
        return;
    }

    let context = unsafe { &*(context as *mut WebViewContext) };

    match &context.handler {
        MixWebviewHnadler::WebViewHandler(handler) => handler.on_fullscreen_change(fullscreen),
        MixWebviewHnadler::WindowlessRenderWebViewHandler(handler) => {
            handler.on_fullscreen_change(fullscreen)
        }
    }
}

extern "C" fn on_message_callback(message: *const c_char, context: *mut c_void) {
    if context.is_null() || message.is_null() {
        return;
    }

    let context = unsafe { &*(context as *mut WebViewContext) };

    if let Ok(message) = unsafe { CStr::from_ptr(message) }.to_str() {
        match &context.handler {
            MixWebviewHnadler::WebViewHandler(handler) => handler.on_message(message),
            MixWebviewHnadler::WindowlessRenderWebViewHandler(handler) => {
                handler.on_message(message)
            }
        }
    }
}

extern "C" fn on_url_change_callback(url: *const c_char, context: *mut c_void) {
    if context.is_null() || url.is_null() {
        return;
    }

    let context = unsafe { &*(context as *mut WebViewContext) };

    if let Ok(url) = unsafe { CStr::from_ptr(url) }.to_str() {
        match &context.handler {
            MixWebviewHnadler::WebViewHandler(handler) => handler.on_url_change(url),
            MixWebviewHnadler::WindowlessRenderWebViewHandler(handler) => {
                handler.on_url_change(url)
            }
        }
    }
}

extern "C" fn on_cursor_callback(ty: sys::CursorType, context: *mut c_void) {
    if context.is_null() {
        return;
    }

    let ty = unsafe { std::mem::transmute::<sys::CursorType, CursorType>(ty) };

    let context = unsafe { &*(context as *mut WebViewContext) };
    match &context.handler {
        MixWebviewHnadler::WebViewHandler(handler) => handler.on_cursor_change(ty),
        MixWebviewHnadler::WindowlessRenderWebViewHandler(handler) => handler.on_cursor_change(ty),
    }
}

extern "C" fn on_js_dialog_callback(
    dialog_type: sys::JSDialogType,
    message_text: *const c_char,
    default_prompt_text: *const c_char,
    context: *mut c_void,
) {
    if context.is_null() {
        return;
    }

    let context = unsafe { &*(context as *mut WebViewContext) };

    let message = if message_text.is_null() {
        ""
    } else {
        match unsafe { CStr::from_ptr(message_text) }.to_str() {
            Ok(s) => s,
            Err(_) => return,
        }
    };

    let prompt = if default_prompt_text.is_null() {
        ""
    } else {
        match unsafe { CStr::from_ptr(default_prompt_text) }.to_str() {
            Ok(s) => s,
            Err(_) => return,
        }
    };

    let dt = dialog_type as u32;
    match &context.handler {
        MixWebviewHnadler::WebViewHandler(handler) => handler.on_js_dialog(dt, message, prompt),
        MixWebviewHnadler::WindowlessRenderWebViewHandler(handler) => {
            handler.on_js_dialog(dt, message, prompt)
        }
    }
}

extern "C" fn on_file_dialog_callback(
    mode: sys::FileDialogMode,
    title: *const c_char,
    default_file_path: *const c_char,
    context: *mut c_void,
) {
    if context.is_null() {
        return;
    }

    let context = unsafe { &*(context as *mut WebViewContext) };

    let title_str = if title.is_null() {
        ""
    } else {
        match unsafe { CStr::from_ptr(title) }.to_str() {
            Ok(s) => s,
            Err(_) => return,
        }
    };

    let path_str = if default_file_path.is_null() {
        ""
    } else {
        match unsafe { CStr::from_ptr(default_file_path) }.to_str() {
            Ok(s) => s,
            Err(_) => return,
        }
    };

    let m = mode as u32;
    match &context.handler {
        MixWebviewHnadler::WebViewHandler(handler) => {
            handler.on_file_dialog(m, title_str, path_str)
        }
        MixWebviewHnadler::WindowlessRenderWebViewHandler(handler) => {
            handler.on_file_dialog(m, title_str, path_str)
        }
    }
}
