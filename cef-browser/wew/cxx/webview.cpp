//
//  webview.cpp
//  webview
//
//  Created by mycrl on 2025/6/19.
//

#include "webview.h"

#include <cstdlib>
#include <filesystem>

/* CefContextMenuHandler */

void IWebViewContextMenu::OnBeforeContextMenu(CefRefPtr<CefBrowser> browser,
                                              CefRefPtr<CefFrame> frame,
                                              CefRefPtr<CefContextMenuParams> params,
                                              CefRefPtr<CefMenuModel> model)
{
    if (params->GetTypeFlags() & (CM_TYPEFLAG_SELECTION | CM_TYPEFLAG_EDITABLE))
    {
        return;
    }

    model->Clear();
}

bool IWebViewContextMenu::OnContextMenuCommand(CefRefPtr<CefBrowser> browser,
                                               CefRefPtr<CefFrame> frame,
                                               CefRefPtr<CefContextMenuParams> params,
                                               int command_id,
                                               EventFlags event_flags)
{
    return false;
};

/* CefLoadHandler */

IWebViewLoad::IWebViewLoad(WebViewHandler &handler) : _handler(handler)
{
}

void IWebViewLoad::OnLoadStart(CefRefPtr<CefBrowser> browser, CefRefPtr<CefFrame> frame, TransitionType transition_type)
{
    _handler.on_state_change(WebViewState::WEW_BEFORE_LOAD, _handler.context);
}

void IWebViewLoad::OnLoadEnd(CefRefPtr<CefBrowser> browser, CefRefPtr<CefFrame> frame, int httpStatusCode)
{
    _handler.on_state_change(WebViewState::WEW_LOADED, _handler.context);
    browser->GetHost()->SetFocus(true);
}

void IWebViewLoad::OnLoadError(CefRefPtr<CefBrowser> browser,
                               CefRefPtr<CefFrame> frame,
                               ErrorCode error_code,
                               const CefString &error_text,
                               const CefString &failed_url)
{
    _handler.on_state_change(WebViewState::WEW_LOAD_ERROR, _handler.context);
}

/* CefLifeSpanHandler */

// clang-format off
IWebViewLifeSpan::IWebViewLifeSpan(std::optional<CefRefPtr<CefBrowser>> &browser, WebViewHandler &handler)
    : _handler(handler)
    , _browser(browser)
{
}
// clang-format on

void IWebViewLifeSpan::OnAfterCreated(CefRefPtr<CefBrowser> browser)
{
    _browser = browser;

    browser->GetHost()->WasResized();
}

bool IWebViewLifeSpan::DoClose(CefRefPtr<CefBrowser> browser)
{
    _handler.on_state_change(WebViewState::WEW_REQUEST_CLOSE, _handler.context);

    return false;
}

bool IWebViewLifeSpan::OnBeforePopup(CefRefPtr<CefBrowser> browser,
                                     CefRefPtr<CefFrame> frame,
                                     int popup_id,
                                     const CefString &target_url,
                                     const CefString &target_frame_name,
                                     CefLifeSpanHandler::WindowOpenDisposition target_disposition,
                                     bool user_gesture,
                                     const CefPopupFeatures &popupFeatures,
                                     CefWindowInfo &windowInfo,
                                     CefRefPtr<CefClient> &client,
                                     CefBrowserSettings &settings,
                                     CefRefPtr<CefDictionaryValue> &extra_info,
                                     bool *no_javascript_access)
{
    browser->GetMainFrame()->LoadURL(target_url);

    return true;
}

void IWebViewLifeSpan::OnBeforeClose(CefRefPtr<CefBrowser> browser)
{
    _browser = std::nullopt;

    _handler.on_state_change(WebViewState::WEW_CLOSE, _handler.context);
}

/* CefDragHandler */

bool IWebViewDrag::OnDragEnter(CefRefPtr<CefBrowser> browser,
                               CefRefPtr<CefDragData> dragData,
                               CefDragHandler::DragOperationsMask mask)
{
    return true;
}

/* CefDisplayHandler */

IWebViewDisplay::IWebViewDisplay(WebViewHandler &handler) : _handler(handler)
{
}

void IWebViewDisplay::OnTitleChange(CefRefPtr<CefBrowser> browser, const CefString &title)
{
    std::string value = title.ToString();
    _handler.on_title_change(value.c_str(), _handler.context);
};

void IWebViewDisplay::OnAddressChange(CefRefPtr<CefBrowser> browser, CefRefPtr<CefFrame> frame, const CefString &url)
{
    if (frame->IsMain() && _handler.on_url_change)
    {
        std::string value = url.ToString();
        _handler.on_url_change(value.c_str(), _handler.context);
    }
}

void IWebViewDisplay::OnFullscreenModeChange(CefRefPtr<CefBrowser> browser, bool fullscreen)
{
    _handler.on_fullscreen_change(fullscreen, _handler.context);
};

bool IWebViewDisplay::OnCursorChange(CefRefPtr<CefBrowser> browser,
                                     CefCursorHandle cursor,
                                     cef_cursor_type_t type,
                                     const CefCursorInfo &custom_cursor_info)
{
    _handler.on_cursor(static_cast<CursorType>(static_cast<int>(type)), _handler.context);

    return true;
}

/* CefRenderHandler */

// clang-format off
IWebViewRender::IWebViewRender(const WebViewSettings *settings, WebViewHandler &handler)
    : _handler(handler)
    , _device_scale_factor(settings->device_scale_factor)
{
    assert(settings != nullptr);

    _view_rect.width = settings->width;
    _view_rect.height = settings->height;
}
// clang-format on

bool IWebViewRender::GetScreenInfo(CefRefPtr<CefBrowser> browser, CefScreenInfo &info)
{
    info.device_scale_factor = _device_scale_factor;

    return true;
}

void IWebViewRender::OnImeCompositionRangeChanged(CefRefPtr<CefBrowser> browser,
                                                  const CefRange &selected_range,
                                                  const RectList &character_bounds)
{
    if (character_bounds.size() == 0)
    {
        return;
    }

    auto first_rect = character_bounds[0];

    Rect rect;
    rect.x = first_rect.x;
    rect.y = first_rect.y;
    rect.width = first_rect.width;
    rect.height = first_rect.height;

    _handler.on_ime_rect(rect, _handler.context);
}

void IWebViewRender::GetViewRect(CefRefPtr<CefBrowser> browser, CefRect &rect)
{
    rect.x = _view_rect.x;
    rect.y = _view_rect.y;
    rect.width = _view_rect.width;
    rect.height = _view_rect.height;
}

void IWebViewRender::OnPaint(CefRefPtr<CefBrowser> browser,
                             PaintElementType type,
                             const RectList &dirtyRects,
                             const void *buffer, // BGRA32
                             int width,
                             int height)
{
    if (buffer == nullptr)
    {
        return;
    }

    Frame frame;
    frame.width = width;
    frame.height = height;
    frame.buffer = buffer;
    frame.is_popup = type == PaintElementType::PET_POPUP;

    auto rect = dirtyRects[0];
    frame.x = frame.is_popup ? _popup_rect.x : rect.x;
    frame.y = frame.is_popup ? _popup_rect.y : rect.y;

    _handler.on_frame(&frame, _handler.context);
}

void IWebViewRender::OnPopupSize(CefRefPtr<CefBrowser> browser, const CefRect &rect)
{
    _popup_rect.x = rect.x;
    _popup_rect.y = rect.y;
    _popup_rect.width = rect.width;
    _popup_rect.height = rect.height;
}

void IWebViewRender::Resize(int width, int height)
{
    _view_rect.width = width;
    _view_rect.height = height;
}

/* CefJSDialogHandler */

IWebViewJSDialog::IWebViewJSDialog(WebViewHandler &handler) : _handler(handler)
{
}

bool IWebViewJSDialog::OnJSDialog(CefRefPtr<CefBrowser> browser,
                                   const CefString &origin_url,
                                   JSDialogType dialog_type,
                                   const CefString &message_text,
                                   const CefString &default_prompt_text,
                                   CefRefPtr<CefJSDialogCallback> callback,
                                   bool &suppress_message)
{
    ::JSDialogType type;
    switch (dialog_type)
    {
    case JSDIALOGTYPE_ALERT:
        type = WEW_JSDIALOG_ALERT;
        break;
    case JSDIALOGTYPE_CONFIRM:
        type = WEW_JSDIALOG_CONFIRM;
        break;
    case JSDIALOGTYPE_PROMPT:
        type = WEW_JSDIALOG_PROMPT;
        break;
    default:
        type = WEW_JSDIALOG_ALERT;
        break;
    }

    std::string msg = message_text.ToString();
    std::string prompt = default_prompt_text.ToString();

    if (_handler.on_js_dialog)
    {
        _handler.on_js_dialog(type, msg.c_str(), prompt.c_str(), _handler.context);
    }

    // Suppress native dialog, auto-respond
    callback->Continue(true, default_prompt_text);
    return true;
}

bool IWebViewJSDialog::OnBeforeUnloadDialog(CefRefPtr<CefBrowser> browser,
                                             const CefString &message_text,
                                             bool is_reload,
                                             CefRefPtr<CefJSDialogCallback> callback)
{
    std::string msg = message_text.ToString();

    if (_handler.on_js_dialog)
    {
        _handler.on_js_dialog(WEW_JSDIALOG_BEFOREUNLOAD, msg.c_str(), "", _handler.context);
    }

    // Allow navigation, suppress native dialog
    callback->Continue(true, CefString());
    return true;
}

void IWebViewJSDialog::OnResetDialogState(CefRefPtr<CefBrowser> browser)
{
}

/* CefDialogHandler */

IWebViewFileDialog::IWebViewFileDialog(WebViewHandler &handler) : _handler(handler)
{
}

bool IWebViewFileDialog::OnFileDialog(CefRefPtr<CefBrowser> browser,
                                       FileDialogMode mode,
                                       const CefString &title,
                                       const CefString &default_file_path,
                                       const std::vector<CefString> &accept_filters,
                                       const std::vector<CefString> &accept_extensions,
                                       const std::vector<CefString> &accept_descriptions,
                                       CefRefPtr<CefFileDialogCallback> callback)
{
    ::FileDialogMode m;
    switch (mode)
    {
    case FILE_DIALOG_OPEN:
        m = WEW_FILE_DIALOG_OPEN;
        break;
    case FILE_DIALOG_OPEN_MULTIPLE:
        m = WEW_FILE_DIALOG_OPEN_MULTIPLE;
        break;
    case FILE_DIALOG_OPEN_FOLDER:
        m = WEW_FILE_DIALOG_OPEN_FOLDER;
        break;
    case FILE_DIALOG_SAVE:
        m = WEW_FILE_DIALOG_SAVE;
        break;
    default:
        m = WEW_FILE_DIALOG_OPEN;
        break;
    }

    std::string t = title.ToString();
    std::string dfp = default_file_path.ToString();

    if (_handler.on_file_dialog)
    {
        _handler.on_file_dialog(m, t.c_str(), dfp.c_str(), _handler.context);
    }

    // Cancel file dialog (suppress native dialog)
    callback->Cancel();
    return true;
}

/* CefDownloadHandler */

IWebViewDownload::IWebViewDownload(WebViewHandler &handler) : _handler(handler) {}

bool IWebViewDownload::OnBeforeDownload(CefRefPtr<CefBrowser> browser,
                                         CefRefPtr<CefDownloadItem> download_item,
                                         const CefString &suggested_name,
                                         CefRefPtr<CefBeforeDownloadCallback> callback)
{
    std::string download_dir = "./downloads";
    const char *env_dir = std::getenv("DOWNLOAD_DIR");
    if (env_dir && std::string(env_dir).length() > 0)
        download_dir = std::string(env_dir);

    std::filesystem::create_directories(download_dir);

    std::string filename = suggested_name.ToString();
    std::string download_path = download_dir + "/" + filename;

    if (_handler.on_download_started)
    {
        uint32_t id = download_item->GetId();
        std::string url = download_item->GetURL().ToString();
        int64_t total = download_item->GetTotalBytes();
        _handler.on_download_started(id, url.c_str(), filename.c_str(), total, _handler.context);
    }

    callback->Continue(download_path, false);
    return true;
}

void IWebViewDownload::OnDownloadUpdated(CefRefPtr<CefBrowser> browser,
                                          CefRefPtr<CefDownloadItem> download_item,
                                          CefRefPtr<CefDownloadItemCallback> callback)
{
    if (_handler.on_download_updated)
    {
        _handler.on_download_updated(
            download_item->GetId(),
            download_item->GetReceivedBytes(),
            download_item->GetTotalBytes(),
            download_item->GetPercentComplete(),
            download_item->IsComplete(),
            download_item->IsCanceled(),
            _handler.context);
    }
}

/* CefRequestHandler */

IWebViewRequest::IWebViewRequest(const WebViewSettings *settings)
    : _handler(new IResourceRequestHandler(settings->request_handler_factory))
{
    assert(settings != nullptr);
}

CefRefPtr<CefResourceRequestHandler> IWebViewRequest::GetResourceRequestHandler(CefRefPtr<CefBrowser> browser,
                                                                                CefRefPtr<CefFrame> frame,
                                                                                CefRefPtr<CefRequest> request,
                                                                                bool is_navigation,
                                                                                bool is_download,
                                                                                const CefString &request_initiator,
                                                                                bool &disable_default_handling)
{
    return _handler;
}

/* IWebView */

IWebView::IWebView(CefSettings &cef_settings, const WebViewSettings *settings, WebViewHandler handler)
    : _handler(handler)
{
    assert(settings != nullptr);

    _drag_handler = new IWebViewDrag();
    _load_handler = new IWebViewLoad(_handler);
    _display_handler = new IWebViewDisplay(_handler);
    _life_span_handler = new IWebViewLifeSpan(_browser, _handler);
    _context_menu_handler = new IWebViewContextMenu();
    _js_dialog_handler = new IWebViewJSDialog(_handler);
    _file_dialog_handler = new IWebViewFileDialog(_handler);
    _download_handler = new IWebViewDownload(_handler);

    if (cef_settings.windowless_rendering_enabled)
    {
        _render_handler = new IWebViewRender(settings, _handler);
    }

    if (settings->request_handler_factory)
    {
        _request_handler = new IWebViewRequest(settings);
    }
}

IWebView::~IWebView()
{
    this->Close();
}

CefRefPtr<CefDragHandler> IWebView::GetDragHandler()
{
    CHECK_REFCOUNTING(nullptr);

    return _drag_handler;
}

CefRefPtr<CefDisplayHandler> IWebView::GetDisplayHandler()
{
    CHECK_REFCOUNTING(nullptr);

    return _display_handler;
}

CefRefPtr<CefLifeSpanHandler> IWebView::GetLifeSpanHandler()
{
    CHECK_REFCOUNTING(nullptr);

    return _life_span_handler;
}

CefRefPtr<CefLoadHandler> IWebView::GetLoadHandler()
{
    CHECK_REFCOUNTING(nullptr);

    return _load_handler;
}

CefRefPtr<CefRenderHandler> IWebView::GetRenderHandler()
{
    CHECK_REFCOUNTING(nullptr);

    return _render_handler;
}

CefRefPtr<CefRequestHandler> IWebView::GetRequestHandler()
{
    CHECK_REFCOUNTING(nullptr);

    return _request_handler;
}

CefRefPtr<CefContextMenuHandler> IWebView::GetContextMenuHandler()
{
    CHECK_REFCOUNTING(nullptr);

    return _context_menu_handler;
}

CefRefPtr<CefJSDialogHandler> IWebView::GetJSDialogHandler()
{
    CHECK_REFCOUNTING(nullptr);

    return _js_dialog_handler;
}

CefRefPtr<CefDialogHandler> IWebView::GetDialogHandler()
{
    CHECK_REFCOUNTING(nullptr);

    return _file_dialog_handler;
}

CefRefPtr<CefDownloadHandler> IWebView::GetDownloadHandler()
{
    CHECK_REFCOUNTING(nullptr);

    return _download_handler;
}

bool IWebView::OnProcessMessageReceived(CefRefPtr<CefBrowser> browser,
                                        CefRefPtr<CefFrame> frame,
                                        CefProcessId source_process,
                                        CefRefPtr<CefProcessMessage> message)
{
    CHECK_REFCOUNTING(false);

    if (!_browser.has_value())
    {
        return false;
    }

    auto args = message->GetArgumentList();
    std::string payload = args->GetString(0);
    _handler.on_message(payload.c_str(), _handler.context);

    return true;
}

void IWebView::SetDevToolsOpenState(bool is_open)
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    if (is_open)
    {
        _browser.value()->GetHost()->ShowDevTools(CefWindowInfo(), nullptr, CefBrowserSettings(), CefPoint());
    }
    else
    {
        _browser.value()->GetHost()->CloseDevTools();
    }
}

RawWindowHandle IWebView::GetWindowHandle()
{
#ifdef LINUX
    CHECK_REFCOUNTING(0);

    return _browser.has_value() ? _browser.value()->GetHost()->GetWindowHandle() : 0;
#else
    CHECK_REFCOUNTING(nullptr);

    return _browser.has_value() ? _browser.value()->GetHost()->GetWindowHandle() : nullptr;
#endif
}

void IWebView::SendMessage(std::string message)
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    auto msg = CefProcessMessage::Create("MESSAGE_TRANSPORT");
    CefRefPtr<CefListValue> args = msg->GetArgumentList();
    args->SetSize(1);
    args->SetString(0, message);
    _browser.value()->GetMainFrame()->SendProcessMessage(PID_RENDERER, msg);
}

void IWebView::Close()
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    _browser.value()->GetHost()->CloseBrowser(true);
    _browser = std::nullopt;

    CLOSE_RUNNING;
}

void IWebView::OnIMEComposition(std::string input)
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    _browser.value()->GetHost()->ImeCommitText(input, CefRange::InvalidRange(), 0);
}

void IWebView::OnIMESetComposition(std::string input, int x, int y)
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    CefCompositionUnderline line;
    line.style = CEF_CUS_DASH;
    line.range = CefRange(0, y);

    _browser.value()->GetHost()->ImeSetComposition(input, {line}, CefRange::InvalidRange(), CefRange(x, y));
}
void IWebView::OnMouseClick(cef_mouse_event_t event, cef_mouse_button_type_t button, bool pressed)
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    _browser.value()->GetHost()->SendMouseClickEvent(event, button, !pressed, 1);
}

void IWebView::OnMouseMove(cef_mouse_event_t event)
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    _browser.value()->GetHost()->SendMouseMoveEvent(event, false);
}

void IWebView::OnMouseWheel(cef_mouse_event_t event, int x, int y)
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    _browser.value()->GetHost()->SendMouseWheelEvent(event, x, y);
}

void IWebView::OnKeyboard(cef_key_event_t event)
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    _browser.value()->GetHost()->SendKeyEvent(event);
}

void IWebView::OnTouch(cef_touch_event_t event)
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    _browser.value()->GetHost()->SendTouchEvent(event);
}

void IWebView::Resize(int width, int height)
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    if (_render_handler != nullptr)
    {
        _render_handler->Resize(width, height);
        _browser.value()->GetHost()->WasResized();
    }
}

void IWebView::SetFocus(bool enable)
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    _browser.value()->GetHost()->SetFocus(enable);
}

void IWebView::Navigate(std::string url)
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    _browser.value()->GetMainFrame()->LoadURL(url);
}

void IWebView::GoBack()
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    _browser.value()->GoBack();
}

void IWebView::GoForward()
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    _browser.value()->GoForward();
}

void IWebView::Reload()
{
    CHECK_REFCOUNTING();

    if (!_browser.has_value())
    {
        return;
    }

    _browser.value()->Reload();
}

std::string IWebView::GetURL()
{
    CHECK_REFCOUNTING("");

    if (!_browser.has_value())
    {
        return "";
    }

    return _browser.value()->GetMainFrame()->GetURL().ToString();
}
