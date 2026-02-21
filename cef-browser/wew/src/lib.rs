//! Wew is a cross-platform WebView rendering library based on Chromium Embedded
//! Framework (CEF). It supports mouse, keyboard, touch, input methods,
//! off-screen rendering, and communication with web pages.
//!
//! ## Thread Considerations
//!
//! In the current project, WebView and Runtime calls are best executed on the
//! UI thread, which is the main thread of the application process.
//!
//! Creating a Runtime must be completed on the UI thread, and all message loop
//! calls must also be operated on the UI thread.
//!
//! Other calls should be executed on the UI thread whenever possible, unless it
//! is truly unavoidable. Although these calls can run on any thread, there is
//! currently no guarantee that they will not cause other side effects.
//!
//! However, it is important to note that if the WebView manages window events
//! on its own, such as not using off-screen rendering, then the WebView can be
//! created on any thread.
//!
//!
//! ## Examples
//!
//! ```no_run
//! use std::{
//!     sync::mpsc::{Sender, channel},
//!     thread,
//! };
//!
//! use wew::{
//!     MainThreadMessageLoop, MessageLoopAbstract, NativeWindowWebView,
//!     runtime::{LogLevel, RuntimeHandler},
//!     webview::{WebViewAttributes, WebViewHandler, WebViewState},
//! };
//!
//! struct RuntimeObserver {
//!     tx: Sender<()>,
//! }
//!
//! impl RuntimeHandler for RuntimeObserver {
//!     fn on_context_initialized(&self) {
//!         self.tx.send(()).unwrap();
//!     }
//! }
//!
//! struct WebViewObserver;
//!
//! impl WebViewHandler for WebViewObserver {
//!     fn on_state_change(&self, state: WebViewState) {
//!         if state == WebViewState::Close {
//!             std::process::exit(0);
//!         }
//!     }
//! }
//!
//! fn main() {
//!     if wew::is_subprocess() {
//!         wew::execute_subprocess();
//!
//!         return;
//!     }
//!
//!     #[cfg(target_os = "macos")]
//!     wew::utils::startup_nsapplication();
//!
//!     let message_loop = MainThreadMessageLoop::default();
//!
//!     let mut runtime_attributes_builder =
//!         message_loop.create_runtime_attributes_builder::<NativeWindowWebView>();
//!
//!     runtime_attributes_builder = runtime_attributes_builder
//!         // Set cache path, here we use environment variables passed by the build script.
//!         .with_root_cache_path(option_env!("CACHE_PATH").unwrap())
//!         .with_cache_path(option_env!("CACHE_PATH").unwrap())
//!         .with_log_severity(LogLevel::Info);
//!
//!     let (tx, rx) = channel();
//!
//!     // Create runtime, wait for the `on_context_initialized` event to be triggered
//!     // before considering the creation successful.
//!     let runtime = runtime_attributes_builder
//!         .build()
//!         .create_runtime(RuntimeObserver { tx })
//!         .unwrap();
//!
//!     thread::spawn(move || {
//!         rx.recv().unwrap();
//!
//!         let webview = runtime
//!             .create_webview(
//!                 "https://www.google.com",
//!                 WebViewAttributes::default(),
//!                 WebViewObserver,
//!             )
//!             .unwrap();
//!
//!         std::mem::forget(webview);
//!         std::mem::forget(runtime);
//!     });
//!
//!     message_loop.block_run();
//! }
//! ```

#![cfg_attr(
    docsrs,
    feature(doc_auto_cfg, doc_cfg_hide),
    doc(cfg_hide(doc, docsrs))
)]
#![allow(clippy::needless_doctest_main)]

pub mod events;
pub mod request;
pub mod runtime;
pub mod utils;
pub mod webview;

use std::sync::atomic::Ordering;

use self::runtime::{RUNTIME_RUNNING, RuntimeAttributesBuilder};

#[cfg(feature = "winit")]
pub use winit;

pub use raw_window_handle;

#[allow(
    dead_code,
    unused_imports,
    non_snake_case,
    non_camel_case_types,
    non_upper_case_globals
)]
mod sys {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

#[derive(Debug)]
pub enum Error {
    /// The current thread is not the main thread.
    NonUIThread,
    FailedToCreateRuntime,
    /// Only one runtime can be created in a process. Repeated creation will
    /// trigger this error.
    RuntimeAlreadyExists,
    /// If the runtime is not initialized, creating WebView and other operations
    /// will trigger this error.
    RuntimeNotInitialization,
    FailedToCreateWebView,
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Represents a rectangular area
#[derive(Debug, Clone, Copy, Default)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Message loop abstraction
///
/// Message loop abstraction, used to implement different message loop types.
pub trait MessageLoopAbstract: Default + Clone + Copy {
    /// Create a runtime attributes builder
    ///
    /// This function is used to create a runtime attributes builder.
    fn create_runtime_attributes_builder<W: Default>(&self) -> RuntimeAttributesBuilder<Self, W> {
        RuntimeAttributesBuilder::<Self, W>::default()
    }
}

/// Multi-threaded message loop
///
/// Using multi-threaded message runtime will create a separate thread inside
/// the runtime to run the message loop.
///
/// Note that macOS does not support this type of message loop.
#[derive(Clone, Copy)]
pub struct MultiThreadMessageLoop;

impl MessageLoopAbstract for MultiThreadMessageLoop {}

impl Default for MultiThreadMessageLoop {
    fn default() -> Self {
        if cfg!(target_os = "macos") {
            panic!("macOS does not support this type of message loop!");
        }

        Self
    }
}

/// Main thread message loop
///
/// You need to manually run the message loop in the main thread of the process.
#[derive(Default, Clone, Copy)]
pub struct MainThreadMessageLoop;

impl MessageLoopAbstract for MainThreadMessageLoop {}

impl MainThreadMessageLoop {
    /// Run the message loop on main thread
    ///
    /// This function is used to run the message loop on main thread.
    ///
    /// Note that this function will block the current thread until the message
    /// loop ends.
    pub fn block_run(&self) {
        if !utils::is_main_thread() {
            panic!("this operation is not allowed in non-main threads!");
        }

        unsafe { sys::run_message_loop() }
    }

    /// Quit the message loop on main thread
    ///
    /// This function is used to quit the message loop on main thread.
    ///
    /// Calling this function will cause **`block_run`** to exit and return.
    pub fn quit(&self) {
        unsafe {
            sys::quit_message_loop();
        }
    }
}

/// Message loop pump
///
/// If you need to integrate with existing message loops, the message pump
/// mechanism provides a way for you to drive the message loop yourself.
#[derive(Default, Clone, Copy)]
pub struct MessagePumpLoop;

impl MessageLoopAbstract for MessagePumpLoop {}

impl MessagePumpLoop {
    /// Drive the message loop pump on main thread
    ///
    /// This function is used to poll the message loop on main thread.
    ///
    /// Note that this function won't block the current thread, external code
    /// needs to drive the message loop pump.
    pub fn poll(&self) {
        if !utils::is_main_thread() {
            panic!("this operation is not allowed in non-main threads!");
        }

        if RUNTIME_RUNNING.load(Ordering::Relaxed) {
            unsafe { sys::poll_message_loop() }
        }
    }
}

/// WebView abstraction
///
/// WebView abstraction, used to implement different WebView types.
pub trait WebViewAbstract: Default {}

/// Off-screen rendering mode
///
/// When using off-screen rendering mode, the WebView will not be displayed on
/// screen, but the rendering results will be pushed through
/// **`WindowlessRenderWebViewHandler::on_frame`**, and you can handle the video
/// frames yourself. Also, in this mode, mouse and keyboard events need to be
/// passed to the WebView by yourself.
#[derive(Default, Clone, Copy)]
pub struct WindowlessRenderWebView;

impl WebViewAbstract for WindowlessRenderWebView {}

/// Native window mode
///
/// When using native window mode, the WebView will create a native window and
/// display it on screen.
#[derive(Default, Clone, Copy)]
pub struct NativeWindowWebView;

impl WebViewAbstract for NativeWindowWebView {}

/// Execute subprocess
///
/// This method is used to start a subprocess in a separate process.
///
/// ## Examples
///
/// ```no_run
/// fn main() {
///     if wew::is_subprocess() {
///         wew::execute_subprocess();
///
///         return;
///     }
/// }
/// ```
///
/// #### Please be careful!
///
/// Do not call this function in an asynchronous runtime, such as tokio,
/// which can lead to unexpected crashes!
pub fn execute_subprocess() -> bool {
    if !utils::is_main_thread() {
        panic!("this operation is not allowed in non-main threads!");
    }

    let args = utils::Args::default();
    (unsafe { sys::execute_subprocess(args.size() as _, args.as_ptr() as _) }) == 0
}

/// Check if current process is a subprocess
///
/// This function is used to check if the current process is a subprocess.
///
/// Note that if the current process is a subprocess, it will block until the
/// subprocess exits.
pub fn is_subprocess() -> bool {
    std::env::args().any(|it| it.contains("--type="))
}
