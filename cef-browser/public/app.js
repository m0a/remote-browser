(function () {
  "use strict";

  var canvas = document.getElementById("screen");
  var ctx = canvas.getContext("2d");
  var statusEl = document.getElementById("status");
  var kbToggle = document.getElementById("kb-toggle");
  var hiddenInput = document.getElementById("hidden-input");
  var urlInput = document.getElementById("url-input");
  var btnBack = document.getElementById("btn-back");
  var btnForward = document.getElementById("btn-forward");
  var btnReload = document.getElementById("btn-reload");
  var urlForm = document.getElementById("url-form");

  var dialogOverlay = document.getElementById("dialog-overlay");

  var ws = null;
  var metadata = null;
  var frameRect = { x: 0, y: 0, width: 0, height: 0 };
  var lastImage = null;
  var connected = false;
  var reconnectTimer = null;
  var dialogTimer = null;
  var cursorPos = null; // {cx, cy} in canvas pixel coords

  // --- Zoom/Pan State ---
  var viewScale = 1;   // 1 = fit-to-screen, up to 5x
  var viewPanX = 0;    // canvas pixel offset from default centered position
  var viewPanY = 0;
  var zoomIndicatorTimer = null;

  function isZoomed() { return viewScale > 1.05; }

  // --- Canvas Setup ---

  function resizeCanvas() {
    var dpr = window.devicePixelRatio || 1;
    var vv = window.visualViewport;
    var w = vv ? vv.width : window.innerWidth;
    var h = vv ? vv.height : window.innerHeight;
    // Subtract toolbar (40px) from available height
    var canvasH = h - 40;
    if (canvasH < 100) canvasH = 100;

    canvas.width = w * dpr;
    canvas.height = canvasH * dpr;
    canvas.style.width = w + "px";
    canvas.style.height = canvasH + "px";
    if (lastImage) drawFrame(lastImage);
  }

  // --- Frame Drawing ---

  function drawFrame(img) {
    lastImage = img;
    var cw = canvas.width;
    var ch = canvas.height;
    var iw = img.naturalWidth || img.width;
    var ih = img.naturalHeight || img.height;

    var baseScale = Math.min(cw / iw, ch / ih);
    var effectiveScale = baseScale * viewScale;
    var dw = iw * effectiveScale;
    var dh = ih * effectiveScale;

    // Default centered position + pan offset
    var dx = (cw - dw) / 2 + viewPanX;
    var dy = (ch - dh) / 2 + viewPanY;

    // Clamp pan so frame edge stays visible
    if (dw > cw) {
      dx = Math.min(0, Math.max(cw - dw, dx));
    } else {
      dx = (cw - dw) / 2;
    }
    if (dh > ch) {
      dy = Math.min(0, Math.max(ch - dh, dy));
    } else {
      dy = (ch - dh) / 2;
    }

    // Write back clamped pan
    viewPanX = dx - (cw - dw) / 2;
    viewPanY = dy - (ch - dh) / 2;

    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, cw, ch);
    ctx.drawImage(img, dx, dy, dw, dh);

    frameRect = { x: dx, y: dy, width: dw, height: dh };

    // Draw cursor
    if (cursorPos) {
      ctx.beginPath();
      ctx.arc(cursorPos.cx, cursorPos.cy, 8, 0, 2 * Math.PI);
      ctx.fillStyle = "rgba(255, 60, 60, 0.7)";
      ctx.fill();
      ctx.lineWidth = 2;
      ctx.strokeStyle = "rgba(255, 255, 255, 0.9)";
      ctx.stroke();
    }

    // Zoom indicator
    if (isZoomed()) {
      var label = viewScale.toFixed(1) + "x";
      ctx.font = "bold 24px -apple-system, sans-serif";
      ctx.fillStyle = "rgba(255,255,255,0.6)";
      ctx.textAlign = "right";
      ctx.fillText(label, cw - 16, ch - 16);
      ctx.textAlign = "start";
    }
  }

  function zoomAtPoint(newScale, cx, cy) {
    newScale = Math.max(1, Math.min(5, newScale));
    if (!lastImage) { viewScale = newScale; return; }

    // Frame-relative position of the zoom center (0-1)
    var fx = (cx - frameRect.x) / frameRect.width;
    var fy = (cy - frameRect.y) / frameRect.height;

    var iw = lastImage.naturalWidth || lastImage.width;
    var ih = lastImage.naturalHeight || lastImage.height;
    var cw = canvas.width;
    var ch = canvas.height;
    var baseScale = Math.min(cw / iw, ch / ih);

    var newW = iw * baseScale * newScale;
    var newH = ih * baseScale * newScale;

    // Keep zoom center pinned: cx = newDx + fx * newW
    var newDx = cx - fx * newW;
    var newDy = cy - fy * newH;

    viewPanX = newDx - (cw - newW) / 2;
    viewPanY = newDy - (ch - newH) / 2;
    viewScale = newScale;
  }

  // --- Coordinate Mapping ---

  function clientToCDP(clientX, clientY) {
    if (!metadata) return null;

    var rect = canvas.getBoundingClientRect();
    var dpr = window.devicePixelRatio || 1;

    // Canvas pixel coordinates
    var cx = (clientX - rect.left) * dpr;
    var cy = (clientY - rect.top) * dpr;

    // Update cursor position for visual feedback
    cursorPos = { cx: cx, cy: cy };
    if (lastImage) drawFrame(lastImage);

    // Map to frame image space (0-1)
    var fx = (cx - frameRect.x) / frameRect.width;
    var fy = (cy - frameRect.y) / frameRect.height;

    // Out of bounds check
    if (fx < 0 || fx > 1 || fy < 0 || fy > 1) return null;

    // Map to CDP viewport coordinates
    return {
      x: Math.round(fx * metadata.deviceWidth),
      y: Math.round(fy * metadata.deviceHeight),
    };
  }

  // --- WebSocket Connection ---

  function connect() {
    if (ws) {
      ws.close();
      ws = null;
    }

    var proto = location.protocol === "https:" ? "wss:" : "ws:";
    ws = new WebSocket(proto + "//" + location.host + "/ws");

    ws.onopen = function () {
      connected = true;
      setStatus("connected");
    };

    ws.binaryType = "arraybuffer";

    ws.onmessage = function (e) {
      if (e.data instanceof ArrayBuffer) {
        // Binary frame: [width:u32le][height:u32le][jpeg...]
        var view = new DataView(e.data);
        var w = view.getUint32(0, true);
        var h = view.getUint32(4, true);
        metadata = { deviceWidth: w, deviceHeight: h };
        var jpegBlob = new Blob([new Uint8Array(e.data, 8)], { type: "image/jpeg" });
        var url = URL.createObjectURL(jpegBlob);
        var img = new Image();
        img.onload = function () {
          drawFrame(img);
          URL.revokeObjectURL(url);
        };
        img.src = url;
        return;
      }

      try {
        var msg = JSON.parse(e.data);
        if (msg.type === "js_dialog") {
          showDialogNotification(msg);
        } else if (msg.type === "file_dialog") {
          showDialogNotification(msg);
        } else if (msg.type === "url") {
          if (document.activeElement !== urlInput) {
            urlInput.value = msg.url;
          }
        } else if (msg.type === "title") {
          document.title = msg.title + " - CEF Remote";
        } else if (msg.type === "webauthn_request") {
          showWebAuthnDialog(msg);
        } else if (msg.type === "error") {
          setStatus("error", msg.message);
        }
      } catch (err) {
        console.error("Message parse error:", err);
      }
    };

    ws.onclose = function () {
      connected = false;
      setStatus("disconnected");
      scheduleReconnect();
    };

    ws.onerror = function () {
      connected = false;
    };

    setStatus("connecting");
  }

  function scheduleReconnect() {
    if (reconnectTimer) return;
    reconnectTimer = setTimeout(function () {
      reconnectTimer = null;
      connect();
    }, 2000);
  }

  function send(msg) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(msg));
    }
  }

  // --- Status Display ---

  function setStatus(state, message) {
    switch (state) {
      case "connecting":
        statusEl.textContent = "Connecting...";
        statusEl.className = "status";
        break;
      case "connected":
        statusEl.textContent = "";
        statusEl.className = "status hidden";
        break;
      case "disconnected":
        statusEl.textContent = "Disconnected. Reconnecting...";
        statusEl.className = "status";
        break;
      case "error":
        statusEl.textContent = message || "Connection error";
        statusEl.className = "status";
        break;
    }
  }

  // --- Touch Events (pinch-zoom, scroll, tap, pan) ---
  //
  // Zoom = 1x (default):
  //   1-finger: tap/drag → sent to remote browser
  //   2-finger: pinch = zoom, parallel move = scroll remote page
  //
  // Zoom > 1x (zoomed in):
  //   1-finger tap (< 10px move): sent to remote as click
  //   1-finger drag (>= 10px move): pan the local view
  //   2-finger: pinch = zoom, parallel move = pan local view
  //   Double-tap: reset zoom to 1x (or zoom to 2x if at 1x)

  var touchState = null;
  var lastTapTime = 0;
  var lastTapX = 0;
  var lastTapY = 0;

  function touchMidpoint(touches) {
    return {
      x: (touches[0].clientX + touches[1].clientX) / 2,
      y: (touches[0].clientY + touches[1].clientY) / 2,
    };
  }

  function touchDistance(touches) {
    var dx = touches[0].clientX - touches[1].clientX;
    var dy = touches[0].clientY - touches[1].clientY;
    return Math.sqrt(dx * dx + dy * dy);
  }

  function startTwoFingerMode(touches) {
    var mid = touchMidpoint(touches);
    var dist = touchDistance(touches);
    touchState = {
      fingers: 2,
      lastX: mid.x,
      lastY: mid.y,
      lastDist: dist,
    };
  }

  function cancelOngoingTouch() {
    if (touchState && touchState.fingers === 1 && touchState.cdpCoords && touchState.sentToRemote) {
      send({
        type: "input_touch",
        eventType: "touchCancel",
        touchPoints: [{ x: touchState.cdpCoords.x, y: touchState.cdpCoords.y, id: 0, radiusX: 1, radiusY: 1, force: 1 }],
      });
    }
  }

  canvas.addEventListener(
    "touchstart",
    function (e) {
      e.preventDefault();

      if (e.touches.length >= 2) {
        // 2+ fingers: enter pinch/scroll/pan mode
        cancelOngoingTouch();
        startTwoFingerMode(e.touches);
      } else if (e.touches.length === 1 && (!touchState || touchState.fingers !== 2)) {
        var t = e.touches[0];
        var coords = clientToCDP(t.clientX, t.clientY);
        if (!coords) return;

        touchState = {
          fingers: 1,
          startX: t.clientX,
          startY: t.clientY,
          lastX: t.clientX,
          lastY: t.clientY,
          cdpCoords: coords,
          startTime: Date.now(),
          isPanning: false,
          sentToRemote: false,
        };

        if (!isZoomed()) {
          // At 1x: immediately send touch to remote
          touchState.sentToRemote = true;
          send({
            type: "input_touch",
            eventType: "touchStart",
            touchPoints: [{ x: coords.x, y: coords.y, id: 0, radiusX: 1, radiusY: 1, force: 1 }],
          });
        }
        // When zoomed: wait to see if it's a tap or pan
      }
    },
    { passive: false }
  );

  canvas.addEventListener(
    "touchmove",
    function (e) {
      e.preventDefault();
      if (!touchState) return;

      if (e.touches.length >= 2) {
        // Upgrade from 1-finger to 2-finger if needed
        if (touchState.fingers === 1) {
          cancelOngoingTouch();
          startTwoFingerMode(e.touches);
          return;
        }

        var mid = touchMidpoint(e.touches);
        var dist = touchDistance(e.touches);
        var moveDx = mid.x - touchState.lastX;
        var moveDy = mid.y - touchState.lastY;
        var dpr = window.devicePixelRatio || 1;

        // Pinch zoom
        if (touchState.lastDist > 0 && dist > 0) {
          var zoomRatio = dist / touchState.lastDist;
          var newScale = viewScale * zoomRatio;
          if (Math.abs(newScale - viewScale) > 0.01) {
            var rect = canvas.getBoundingClientRect();
            var cx = (mid.x - rect.left) * dpr;
            var cy = (mid.y - rect.top) * dpr;
            zoomAtPoint(newScale, cx, cy);
          }
        }

        // Pan (when zoomed) or scroll remote page (when at 1x)
        if (isZoomed()) {
          viewPanX += moveDx * dpr;
          viewPanY += moveDy * dpr;
          if (lastImage) drawFrame(lastImage);
        } else {
          // Reset pan when back to 1x
          viewPanX = 0;
          viewPanY = 0;
          if (lastImage) drawFrame(lastImage);
          // Scroll the remote page
          var scaleY = metadata ? metadata.deviceHeight / (frameRect.height / dpr) : 1;
          var scaleX = metadata ? metadata.deviceWidth / (frameRect.width / dpr) : 1;
          var midCoords = clientToCDP(mid.x, mid.y);
          send({
            type: "input_scroll",
            x: midCoords ? midCoords.x : 0,
            y: midCoords ? midCoords.y : 0,
            deltaX: Math.round(moveDx * scaleX),
            deltaY: Math.round(moveDy * scaleY),
          });
        }

        touchState.lastX = mid.x;
        touchState.lastY = mid.y;
        touchState.lastDist = dist;

      } else if (touchState.fingers === 1 && e.touches.length === 1) {
        var t = e.touches[0];

        if (isZoomed()) {
          // When zoomed: check if this is a pan gesture
          var moveDist = Math.sqrt(
            Math.pow(t.clientX - touchState.startX, 2) +
            Math.pow(t.clientY - touchState.startY, 2)
          );
          if (moveDist > 10 || touchState.isPanning) {
            touchState.isPanning = true;
            var dpr = window.devicePixelRatio || 1;
            viewPanX += (t.clientX - touchState.lastX) * dpr;
            viewPanY += (t.clientY - touchState.lastY) * dpr;
            if (lastImage) drawFrame(lastImage);
          }
          touchState.lastX = t.clientX;
          touchState.lastY = t.clientY;
        } else {
          // At 1x: send drag to remote
          var coords = clientToCDP(t.clientX, t.clientY);
          if (coords) {
            send({
              type: "input_touch",
              eventType: "touchMove",
              touchPoints: [{ x: coords.x, y: coords.y, id: 0, radiusX: 1, radiusY: 1, force: 1 }],
            });
          }
        }
      }
    },
    { passive: false }
  );

  canvas.addEventListener(
    "touchend",
    function (e) {
      e.preventDefault();
      if (!touchState) return;

      if (touchState.fingers === 1 && e.touches.length === 0) {
        var t = e.changedTouches[0];

        if (isZoomed()) {
          if (!touchState.isPanning) {
            // It was a tap while zoomed → send click to remote
            var coords = clientToCDP(t.clientX, t.clientY);
            if (coords) {
              send({
                type: "input_touch",
                eventType: "touchStart",
                touchPoints: [{ x: coords.x, y: coords.y, id: 0, radiusX: 1, radiusY: 1, force: 1 }],
              });
              send({
                type: "input_touch",
                eventType: "touchEnd",
                touchPoints: [{ x: coords.x, y: coords.y, id: 0, radiusX: 1, radiusY: 1, force: 1 }],
              });
            }
          }
          // else: was a pan gesture, no click
        } else {
          // At 1x: send touchEnd
          var coords = clientToCDP(t.clientX, t.clientY);
          if (coords) {
            send({
              type: "input_touch",
              eventType: "touchEnd",
              touchPoints: [{ x: coords.x, y: coords.y, id: 0, radiusX: 1, radiusY: 1, force: 1 }],
            });
          }
        }

        // Double-tap detection
        var now = Date.now();
        var tapDist = Math.sqrt(
          Math.pow(t.clientX - lastTapX, 2) +
          Math.pow(t.clientY - lastTapY, 2)
        );
        if (!touchState.isPanning && now - lastTapTime < 300 && tapDist < 40) {
          // Double tap
          var dpr = window.devicePixelRatio || 1;
          var rect = canvas.getBoundingClientRect();
          var cx = (t.clientX - rect.left) * dpr;
          var cy = (t.clientY - rect.top) * dpr;
          if (isZoomed()) {
            viewScale = 1;
            viewPanX = 0;
            viewPanY = 0;
          } else {
            zoomAtPoint(2, cx, cy);
          }
          if (lastImage) drawFrame(lastImage);
          lastTapTime = 0;
        } else {
          lastTapTime = now;
          lastTapX = t.clientX;
          lastTapY = t.clientY;
        }

        touchState = null;

      } else if (touchState.fingers === 2 && e.touches.length <= 1) {
        // Pinch/scroll gesture ended
        // Snap to 1x if very close
        if (viewScale < 1.05) {
          viewScale = 1;
          viewPanX = 0;
          viewPanY = 0;
          if (lastImage) drawFrame(lastImage);
        }
        touchState = null;
      }
    },
    { passive: false }
  );

  canvas.addEventListener(
    "touchcancel",
    function (e) {
      e.preventDefault();
      touchState = null;
    },
    { passive: false }
  );

  // --- Mouse Events (desktop) ---

  var mouseDown = false;

  canvas.addEventListener("mousedown", function (e) {
    mouseDown = true;
    var coords = clientToCDP(e.clientX, e.clientY);
    if (coords) {
      send({
        type: "input_mouse",
        eventType: "mousePressed",
        x: coords.x,
        y: coords.y,
        button: "left",
        clickCount: 1,
        buttons: 1,
      });
    }
  });

  canvas.addEventListener("mousemove", function (e) {
    var coords = clientToCDP(e.clientX, e.clientY);
    if (coords) {
      send({
        type: "input_mouse",
        eventType: "mouseMoved",
        x: coords.x,
        y: coords.y,
        button: mouseDown ? "left" : "none",
        buttons: mouseDown ? 1 : 0,
      });
    }
  });

  canvas.addEventListener("mouseup", function (e) {
    mouseDown = false;
    var coords = clientToCDP(e.clientX, e.clientY);
    if (coords) {
      send({
        type: "input_mouse",
        eventType: "mouseReleased",
        x: coords.x,
        y: coords.y,
        button: "left",
        clickCount: 1,
      });
    }
  });

  // Mouse wheel → scroll
  canvas.addEventListener(
    "wheel",
    function (e) {
      e.preventDefault();
      var coords = clientToCDP(e.clientX, e.clientY);
      if (coords) {
        send({
          type: "input_scroll",
          x: coords.x,
          y: coords.y,
          deltaX: e.deltaX,
          deltaY: e.deltaY,
        });
      }
    },
    { passive: false }
  );

  // --- Soft Keyboard Toggle ---

  var kbVisible = false;

  kbToggle.addEventListener("click", function (e) {
    e.stopPropagation();
    kbVisible = !kbVisible;
    if (kbVisible) {
      hiddenInput.focus();
      kbToggle.classList.add("active");
    } else {
      hiddenInput.blur();
      kbToggle.classList.remove("active");
    }
  });

  // IME composition tracking
  var composing = false;

  hiddenInput.addEventListener("compositionstart", function () {
    composing = true;
  });

  hiddenInput.addEventListener("compositionend", function (e) {
    composing = false;
    if (e.data) {
      send({ type: "input_text", text: e.data });
    }
    hiddenInput.value = "";
  });

  // Text input from soft keyboard
  hiddenInput.addEventListener("input", function (e) {
    if (composing) return; // Wait for compositionend for IME input
    if (e.inputType === "deleteContentBackward") {
      send({
        type: "input_key",
        eventType: "rawKeyDown",
        key: "Backspace",
        code: "Backspace",
        keyCode: 8,
      });
      send({
        type: "input_key",
        eventType: "keyUp",
        key: "Backspace",
        code: "Backspace",
        keyCode: 8,
      });
    } else if (e.data) {
      send({ type: "input_text", text: e.data });
    }
    hiddenInput.value = "";
  });

  // Special keys from soft keyboard
  hiddenInput.addEventListener("keydown", function (e) {
    if (composing) return; // Don't intercept keys during IME composition

    var specialKeys = {
      Backspace: 8,
      Tab: 9,
      Enter: 13,
      Escape: 27,
      ArrowLeft: 37,
      ArrowUp: 38,
      ArrowRight: 39,
      ArrowDown: 40,
      Delete: 46,
    };

    if (specialKeys[e.key] !== undefined) {
      e.preventDefault();
      send({
        type: "input_key",
        eventType: "rawKeyDown",
        key: e.key,
        code: e.code,
        keyCode: specialKeys[e.key],
      });
      if (e.key === "Enter") {
        send({
          type: "input_key",
          eventType: "char",
          key: e.key,
          code: e.code,
          text: "\r",
          keyCode: 13,
        });
      }
      send({
        type: "input_key",
        eventType: "keyUp",
        key: e.key,
        code: e.code,
        keyCode: specialKeys[e.key],
      });
    }
  });

  hiddenInput.addEventListener("blur", function () {
    if (kbVisible) {
      kbToggle.classList.remove("active");
      kbVisible = false;
    }
  });

  // --- Toolbar Controls ---

  btnBack.addEventListener("click", function (e) {
    e.stopPropagation();
    send({ type: "go_back" });
  });

  btnForward.addEventListener("click", function (e) {
    e.stopPropagation();
    send({ type: "go_forward" });
  });

  btnReload.addEventListener("click", function (e) {
    e.stopPropagation();
    send({ type: "reload" });
  });

  urlForm.addEventListener("submit", function (e) {
    e.preventDefault();
    var url = urlInput.value.trim();
    if (url && url.indexOf("://") === -1 && url.indexOf(".") !== -1) {
      url = "https://" + url;
    }
    if (url) {
      send({ type: "navigate", url: url });
      urlInput.blur();
    }
  });

  // Prevent keyboard events from reaching the canvas when URL input is focused
  urlInput.addEventListener("keydown", function (e) {
    e.stopPropagation();
  });
  urlInput.addEventListener("keyup", function (e) {
    e.stopPropagation();
  });

  // --- Desktop Keyboard (physical) ---

  function getModifiers(e) {
    var m = 0;
    if (e.altKey) m |= 1;
    if (e.ctrlKey) m |= 2;
    if (e.metaKey) m |= 4;
    if (e.shiftKey) m |= 8;
    return m;
  }

  document.addEventListener("keydown", function (e) {
    if (document.activeElement === hiddenInput || document.activeElement === urlInput) return;
    if (!connected) return;

    e.preventDefault();
    var keyCode = e.keyCode || e.which;

    if (e.key.length === 1) {
      // Printable character
      send({
        type: "input_key",
        eventType: "keyDown",
        key: e.key,
        code: e.code,
        text: e.key,
        keyCode: keyCode,
        modifiers: getModifiers(e),
      });
    } else {
      // Special key
      send({
        type: "input_key",
        eventType: "rawKeyDown",
        key: e.key,
        code: e.code,
        keyCode: keyCode,
        modifiers: getModifiers(e),
      });
    }
  });

  document.addEventListener("keyup", function (e) {
    if (document.activeElement === hiddenInput || document.activeElement === urlInput) return;
    if (!connected) return;

    e.preventDefault();
    send({
      type: "input_key",
      eventType: "keyUp",
      key: e.key,
      code: e.code,
      keyCode: e.keyCode || e.which,
      modifiers: getModifiers(e),
    });
  });

  // --- Prevent default behaviors ---

  document.addEventListener("contextmenu", function (e) {
    e.preventDefault();
  });

  // --- Dialog Notification ---

  function showDialogNotification(msg) {
    if (dialogTimer) {
      clearTimeout(dialogTimer);
    }

    var icon, label, detail;
    if (msg.type === "js_dialog") {
      switch (msg.dialogType) {
        case "alert":
          icon = "\u26A0"; label = "Alert"; break;
        case "confirm":
          icon = "\u2753"; label = "Confirm (auto: OK)"; break;
        case "prompt":
          icon = "\u270F"; label = "Prompt (auto: " + (msg.defaultPrompt || "empty") + ")"; break;
        case "beforeunload":
          icon = "\u21A9"; label = "Before Unload (auto: allow)"; break;
        default:
          icon = "\u2139"; label = msg.dialogType; break;
      }
      detail = msg.message || "";
    } else if (msg.type === "file_dialog") {
      icon = "\uD83D\uDCC1";
      label = "File Dialog: " + (msg.mode || "open") + " (blocked)";
      detail = msg.title || "";
    } else {
      return;
    }

    dialogOverlay.innerHTML =
      '<div class="dialog-icon">' + icon + '</div>' +
      '<div class="dialog-body">' +
        '<div class="dialog-label">' + label + '</div>' +
        (detail ? '<div class="dialog-detail">' + escapeHtml(detail) + '</div>' : '') +
      '</div>';
    dialogOverlay.classList.add("visible");

    dialogTimer = setTimeout(function () {
      dialogOverlay.classList.remove("visible");
      dialogTimer = null;
    }, 4000);
  }

  // --- WebAuthn Dialog ---

  function showWebAuthnDialog(msg) {
    // Immediately send cancel to unblock the page
    send({ type: "webauthn_response", action: "cancel" });

    // Show notification (auto-dismiss, non-blocking)
    if (dialogTimer) { clearTimeout(dialogTimer); dialogTimer = null; }

    var rpId = msg.rpId || "this site";
    dialogOverlay.innerHTML =
      '<div class="dialog-icon">\uD83D\uDD11</div>' +
      '<div class="dialog-body">' +
        '<div class="dialog-label">パスキーを自動キャンセルしました</div>' +
        '<div class="dialog-detail">' + escapeHtml(rpId) + ' がパスキーを要求しました。「別の方法を試す」からパスワード認証へ進んでください。</div>' +
      '</div>';

    dialogOverlay.classList.add("visible");
    dialogOverlay.style.pointerEvents = "none";

    dialogTimer = setTimeout(function () {
      dialogOverlay.classList.remove("visible");
      dialogTimer = null;
    }, 5000);
  }

  function escapeHtml(s) {
    var el = document.createElement("span");
    el.textContent = s;
    return el.innerHTML;
  }

  // --- Service Worker ---

  if ("serviceWorker" in navigator) {
    navigator.serviceWorker.register("/sw.js").catch(function () {});
  }

  // --- Init ---

  window.addEventListener("resize", resizeCanvas);
  if (window.visualViewport) {
    window.visualViewport.addEventListener("resize", resizeCanvas);
  }
  resizeCanvas();
  connect();
})();
