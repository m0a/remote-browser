//
//  subprocess.cpp
//  webview
//
//  Created by mycrl on 2025/6/19.
//

#include "subprocess.h"

CefRefPtr<CefRenderProcessHandler> ISubProcess::GetRenderProcessHandler()
{
    return this;
}

void ISubProcess::OnRegisterCustomSchemes(CefRawPtr<CefSchemeRegistrar> registrar)
{
    auto cmd = CefCommandLine::GetGlobalCommandLine();
    if (cmd->HasSwitch("scheme-name"))
    {
        registrar->AddCustomScheme(cmd->GetSwitchValue("scheme-name"),
                                   CEF_SCHEME_OPTION_STANDARD | CEF_SCHEME_OPTION_CORS_ENABLED |
                                       CEF_SCHEME_OPTION_FETCH_ENABLED);
    }
}

void ISubProcess::OnContextCreated(CefRefPtr<CefBrowser> browser,
                                   CefRefPtr<CefFrame> frame,
                                   CefRefPtr<CefV8Context> context)
{
    _sender->SetBrowser(browser);

    CefRefPtr<CefV8Value> native = CefV8Value::CreateObject(nullptr, nullptr);
    native->SetValue("send", CefV8Value::CreateFunction("send", _sender), V8_PROPERTY_ATTRIBUTE_NONE);
    native->SetValue("on", CefV8Value::CreateFunction("on", _receiver), V8_PROPERTY_ATTRIBUTE_NONE);

    CefRefPtr<CefV8Value> global = context->GetGlobal();
    global->SetValue("MessageTransport", std::move(native), V8_PROPERTY_ATTRIBUTE_NONE);

    // WebAuthn monkey-patch: intercept navigator.credentials.get/create
    // to prevent native passkey dialogs that block OSR interaction.
    if (frame->IsMain())
    {
        std::string script = R"(
            (function() {
                if (!navigator.credentials) return;

                navigator.credentials.get = function(options) {
                    if (!options || !options.publicKey) {
                        return navigator.credentials.__proto__.get.call(navigator.credentials, options);
                    }
                    // Notify viewer (fire-and-forget)
                    if (window.MessageTransport) {
                        try {
                            window.MessageTransport.send(JSON.stringify({
                                type: 'webauthn_request',
                                action: 'get',
                                rpId: options.publicKey.rpId || window.location.hostname
                            }));
                        } catch(e) {}
                    }
                    // Immediately reject with NotAllowedError
                    return Promise.reject(new DOMException('The operation was cancelled.', 'NotAllowedError'));
                };

                navigator.credentials.create = function(options) {
                    if (!options || !options.publicKey) {
                        return navigator.credentials.__proto__.create.call(navigator.credentials, options);
                    }
                    if (window.MessageTransport) {
                        try {
                            window.MessageTransport.send(JSON.stringify({
                                type: 'webauthn_request',
                                action: 'create',
                                rpId: (options.publicKey.rp && options.publicKey.rp.id) || window.location.hostname
                            }));
                        } catch(e) {}
                    }
                    return Promise.reject(new DOMException('The operation was cancelled.', 'NotAllowedError'));
                };
            })();
        )";
        frame->ExecuteJavaScript(script, frame->GetURL(), 0);
    }
}

bool ISubProcess::OnProcessMessageReceived(CefRefPtr<CefBrowser> browser,
                                           CefRefPtr<CefFrame> frame,
                                           CefProcessId source_process,
                                           CefRefPtr<CefProcessMessage> message)
{
    auto args = message->GetArgumentList();
    std::string payload = args->GetString(0);
    _receiver->Recv(payload);

    return true;
}

bool MessageSender::Execute(const CefString &name,
                            CefRefPtr<CefV8Value> object,
                            const CefV8ValueList &arguments,
                            CefRefPtr<CefV8Value> &retval,
                            CefString &exception)
{
    if (_browser.has_value() && arguments.size() == 1 && arguments[0]->IsString())
    {
        CefRefPtr<CefV8Context> context = CefV8Context::GetCurrentContext();
        std::string message = arguments[0]->GetStringValue();

        auto msg = CefProcessMessage::Create("MESSAGE_TRANSPORT");
        CefRefPtr<CefListValue> args = msg->GetArgumentList();
        args->SetSize(1);
        args->SetString(0, message);

        _browser.value()->GetMainFrame()->SendProcessMessage(PID_BROWSER, msg);
        retval = CefV8Value::CreateUndefined();

        return true;
    }
    else
    {
        return false;
    }
}

bool MessageReceiver::Execute(const CefString &name,
                              CefRefPtr<CefV8Value> object,
                              const CefV8ValueList &arguments,
                              CefRefPtr<CefV8Value> &retval,
                              CefString &exception)
{
    if (arguments.size() == 1 && arguments[0]->IsFunction())
    {
        _context = std::optional(CefV8Context::GetCurrentContext());
        _callback = std::optional(arguments[0]);
        retval = CefV8Value::CreateUndefined();

        return true;
    }
    else
    {
        return false;
    }
}

void MessageReceiver::Recv(std::string message)
{
    if (_context.has_value() && _callback.has_value())
    {
        _context.value()->Enter();
        CefV8ValueList arguments;
        arguments.push_back(CefV8Value::CreateString(message));
        _callback.value()->ExecuteFunction(nullptr, arguments);
        _context.value()->Exit();
    }
}
