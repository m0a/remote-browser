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
  var debugEl = document.getElementById("debug-info");
  var tabsEl = document.getElementById("tabs");
  var tabNewBtn = document.getElementById("tab-new");
  var sessions = [];
  var activeSessionId = null;

  var ws = null;
  var metadata = null;
  var frameRect = { x: 0, y: 0, width: 0, height: 0 };
  var lastImage = null;
  var connected = false;
  var reconnectTimer = null;
  var disconnectDisplayTimer = null;
  var dialogTimer = null;
  var cursorPos = null; // {cx, cy} in canvas pixel coords
  var audioCtx = null;
  var audioNextTime = 0;

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
    var canvasH = h - 72;
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

  // Draw only the dirty (changed) region onto the canvas
  function drawDirtyRect(img, dx, dy, dw, dh) {
    if (!lastImage || !metadata) return;

    var cw = canvas.width;
    var ch = canvas.height;
    var iw = metadata.deviceWidth;
    var ih = metadata.deviceHeight;

    var baseScale = Math.min(cw / iw, ch / ih);
    var effectiveScale = baseScale * viewScale;

    // Frame position on canvas
    var frameW = iw * effectiveScale;
    var frameH = ih * effectiveScale;
    var frameX = (cw - frameW) / 2 + viewPanX;
    var frameY = (ch - frameH) / 2 + viewPanY;

    // Clamp (same as drawFrame)
    if (frameW > cw) { frameX = Math.min(0, Math.max(cw - frameW, frameX)); }
    else { frameX = (cw - frameW) / 2; }
    if (frameH > ch) { frameY = Math.min(0, Math.max(ch - frameH, frameY)); }
    else { frameY = (ch - frameH) / 2; }

    // Map dirty rect from device coords to canvas coords
    var cx1 = frameX + dx * effectiveScale;
    var cy1 = frameY + dy * effectiveScale;
    var cw1 = dw * effectiveScale;
    var ch1 = dh * effectiveScale;

    // Draw the dirty image at the correct position
    ctx.drawImage(img, cx1, cy1, cw1, ch1);

    // Debug: draw white border around dirty rect
    ctx.strokeStyle = "rgba(255, 255, 255, 0.6)";
    ctx.lineWidth = 1;
    ctx.strokeRect(cx1, cy1, cw1, ch1);
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
    var wsUrl = proto + "//" + location.host + "/ws";
    if (activeSessionId) wsUrl += "?session=" + activeSessionId;
    ws = new WebSocket(wsUrl);

    ws.onopen = function () {
      connected = true;
      if (disconnectDisplayTimer) { clearTimeout(disconnectDisplayTimer); disconnectDisplayTimer = null; }
      setStatus("connected");
    };

    ws.binaryType = "arraybuffer";

    ws.onmessage = function (e) {
      if (e.data instanceof ArrayBuffer) {
        var firstByte = new Uint8Array(e.data, 0, 1)[0];

        if (firstByte === 0x01) {
          // Video frame: [0x01][w:u32][h:u32][dx:u32][dy:u32][dw:u32][dh:u32][webp...]
          var view = new DataView(e.data, 1);
          var w = view.getUint32(0, true);
          var h = view.getUint32(4, true);
          var dx = view.getUint32(8, true);
          var dy = view.getUint32(12, true);
          var dw = view.getUint32(16, true);
          var dh = view.getUint32(20, true);
          metadata = { deviceWidth: w, deviceHeight: h };
          var isFull = (dx === 0 && dy === 0 && dw === w && dh === h);
          var imgBlob = new Blob([new Uint8Array(e.data, 25)], { type: "image/webp" });
          var blobUrl = URL.createObjectURL(imgBlob);
          var img = new Image();
          img.onload = function () {
            if (isFull) {
              drawFrame(img);
            } else {
              drawDirtyRect(img, dx, dy, dw, dh);
            }
            URL.revokeObjectURL(blobUrl);
          };
          img.src = blobUrl;
        } else if (firstByte === 0x02) {
          handleAudioPacket(e.data);
        }
        return;
      }

      try {
        var msg = JSON.parse(e.data);
        showDebug("ws: " + msg.type);
        if (msg.type === "js_dialog") {
          showDialogNotification(msg);
        } else if (msg.type === "file_dialog") {
          showDialogNotification(msg);
        } else if (msg.type === "url") {
          if (document.activeElement !== urlInput) {
            urlInput.value = msg.url;
          }
          var s = sessions.find(function(x) { return x.id === activeSessionId; });
          if (s) { s.url = msg.url; renderTabs(); }
        } else if (msg.type === "title") {
          document.title = msg.title + " - CEF Remote";
          var s = sessions.find(function(x) { return x.id === activeSessionId; });
          if (s) { s.title = msg.title; renderTabs(); }
        } else if (msg.type === "webauthn_request") {
          showWebAuthnDialog(msg);
        } else if (msg.type === "download_started") {
          showDownloadNotification(msg);
        } else if (msg.type === "download_updated") {
          if (msg.isComplete || msg.isCancelled) {
            showDownloadCompleteNotification(msg);
          }
        } else if (msg.type === "audio_started") {
          showDebug("Audio: " + msg.sampleRate + "Hz " + msg.channels + "ch");
        } else if (msg.type === "audio_stopped") {
          audioNextTime = 0;
          showDebug("Audio stopped");
        } else if (msg.type === "error") {
          setStatus("error", msg.message);
        }
      } catch (err) {
        console.error("Message parse error:", err);
      }
    };

    ws.onclose = function () {
      connected = false;
      // Delay showing disconnected status to avoid flashing on brief reconnects
      if (disconnectDisplayTimer) clearTimeout(disconnectDisplayTimer);
      disconnectDisplayTimer = setTimeout(function () {
        if (!connected) setStatus("disconnected");
      }, 1500);
      scheduleReconnect();
    };

    ws.onerror = function () {
      connected = false;
    };

    // Don't show "connecting" status to avoid flashing on quick reconnects
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
  var debugTimer = null;

  function showDebug(text) {
    debugEl.textContent = text;
    if (debugTimer) clearTimeout(debugTimer);
    debugTimer = setTimeout(function() { debugEl.textContent = ""; }, 3000);
  }
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
      initialDist: dist,
      mode: null, // 'pinch' or 'scroll', decided after movement
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
      initAudio();

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
          isScrolling: false,
          sentToRemote: false,
        };
        showDebug("1f: start");
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

        // Determine gesture mode on first significant movement
        if (touchState.mode === null) {
          var distChange = Math.abs(dist - touchState.initialDist);
          var posChange = Math.sqrt(moveDx * moveDx + moveDy * moveDy);
          if (distChange > 20) {
            touchState.mode = "pinch";
            showDebug("2f: pinch");
          } else if (posChange > 8) {
            touchState.mode = "scroll";
            showDebug("2f: scroll");
          }
          // Not enough movement yet — skip
        }

        if (touchState.mode === "pinch") {
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
          if (isZoomed()) {
            viewPanX += moveDx * dpr;
            viewPanY += moveDy * dpr;
          }
          if (lastImage) drawFrame(lastImage);
        } else if (touchState.mode === "scroll") {
          if (isZoomed()) {
            // Pan local view + scroll remote page
            viewPanX += moveDx * dpr;
            viewPanY += moveDy * dpr;
            if (lastImage) drawFrame(lastImage);
          }
          {
            // Scroll the remote page (negate: finger up = scroll down)
            var scaleY = metadata ? metadata.deviceHeight / (frameRect.height / dpr) : 1;
            var scaleX = metadata ? metadata.deviceWidth / (frameRect.width / dpr) : 1;
            var midCoords = clientToCDP(mid.x, mid.y);
            var scrollMsg = {
              type: "input_scroll",
              x: midCoords ? midCoords.x : 0,
              y: midCoords ? midCoords.y : 0,
              deltaX: Math.round(-moveDx * scaleX * 3),
              deltaY: Math.round(-moveDy * scaleY * 3),
            };
            console.log("[scroll] dx=" + scrollMsg.deltaX + " dy=" + scrollMsg.deltaY);
            send(scrollMsg);
          }
        }

        touchState.lastX = mid.x;
        touchState.lastY = mid.y;
        touchState.lastDist = dist;

      } else if (touchState.fingers === 1 && e.touches.length === 1) {
        var t = e.touches[0];
        var moveDist = Math.sqrt(
          Math.pow(t.clientX - touchState.startX, 2) +
          Math.pow(t.clientY - touchState.startY, 2)
        );

        if (isZoomed()) {
          // When zoomed: pan local view + scroll remote page
          if (moveDist > 10 || touchState.isPanning) {
            touchState.isPanning = true;
            var dpr = window.devicePixelRatio || 1;
            var dx = t.clientX - touchState.lastX;
            var dy = t.clientY - touchState.lastY;
            viewPanX += dx * dpr;
            viewPanY += dy * dpr;
            if (lastImage) drawFrame(lastImage);
            // Also scroll the remote page
            var scaleY = metadata ? metadata.deviceHeight / (frameRect.height / dpr) : 1;
            var scaleX = metadata ? metadata.deviceWidth / (frameRect.width / dpr) : 1;
            var scrollCoords = clientToCDP(t.clientX, t.clientY);
            send({
              type: "input_scroll",
              x: scrollCoords ? scrollCoords.x : 0,
              y: scrollCoords ? scrollCoords.y : 0,
              deltaX: Math.round(-dx * scaleX * 3),
              deltaY: Math.round(-dy * scaleY * 3),
            });
          }
        } else {
          // At 1x: scroll if dragged, otherwise wait for tap
          if (moveDist > 10 || touchState.isScrolling) {
            if (!touchState.isScrolling) {
              // Cancel any touch we sent to remote
              if (touchState.sentToRemote) {
                cancelOngoingTouch();
                touchState.sentToRemote = false;
              }
              touchState.isScrolling = true;
              showDebug("1f: scroll mode");
            }
            // Send scroll event
            var dpr = window.devicePixelRatio || 1;
            var scaleY = metadata ? metadata.deviceHeight / (frameRect.height / dpr) : 1;
            var scaleX = metadata ? metadata.deviceWidth / (frameRect.width / dpr) : 1;
            var dx = t.clientX - touchState.lastX;
            var dy = t.clientY - touchState.lastY;
            var scrollCoords = clientToCDP(t.clientX, t.clientY);
            send({
              type: "input_scroll",
              x: scrollCoords ? scrollCoords.x : 0,
              y: scrollCoords ? scrollCoords.y : 0,
              deltaX: Math.round(-dx * scaleX * 3),
              deltaY: Math.round(-dy * scaleY * 3),
            });
          }
        }
        touchState.lastX = t.clientX;
        touchState.lastY = t.clientY;
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

        if (!touchState.isPanning && !touchState.isScrolling) {
          showDebug("1f: tap → click");
          // It was a tap → send click to remote
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
        // else: was a scroll/pan gesture, no click

        // Double-tap detection
        var now = Date.now();
        var tapDist = Math.sqrt(
          Math.pow(t.clientX - lastTapX, 2) +
          Math.pow(t.clientY - lastTapY, 2)
        );
        if (!touchState.isPanning && !touchState.isScrolling && now - lastTapTime < 300 && tapDist < 40) {
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
    canvas.focus();
    initAudio();
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

  // --- Download Notifications ---

  function showDownloadNotification(msg) {
    if (dialogTimer) { clearTimeout(dialogTimer); dialogTimer = null; }
    var sizeStr = msg.totalBytes > 0 ? " (" + formatBytes(msg.totalBytes) + ")" : "";
    dialogOverlay.innerHTML =
      '<div class="dialog-icon">\u2B07</div>' +
      '<div class="dialog-body">' +
        '<div class="dialog-label">Download started</div>' +
        '<div class="dialog-detail">' + escapeHtml(msg.filename || "file") + sizeStr + '</div>' +
      '</div>';
    dialogOverlay.classList.add("visible");
    dialogOverlay.style.pointerEvents = "none";
    dialogTimer = setTimeout(function () {
      dialogOverlay.classList.remove("visible");
      dialogTimer = null;
    }, 4000);
  }

  function showDownloadCompleteNotification(msg) {
    if (dialogTimer) { clearTimeout(dialogTimer); dialogTimer = null; }
    var icon = msg.isCancelled ? "\u274C" : "\u2705";
    var label = msg.isCancelled ? "Download cancelled" : "Download complete";
    var sizeStr = msg.totalBytes > 0 ? " (" + formatBytes(msg.totalBytes) + ")" : "";
    dialogOverlay.innerHTML =
      '<div class="dialog-icon">' + icon + '</div>' +
      '<div class="dialog-body">' +
        '<div class="dialog-label">' + label + '</div>' +
        '<div class="dialog-detail">' + escapeHtml("ID: " + msg.id) + sizeStr + '</div>' +
      '</div>';
    dialogOverlay.classList.add("visible");
    dialogOverlay.style.pointerEvents = "none";
    dialogTimer = setTimeout(function () {
      dialogOverlay.classList.remove("visible");
      dialogTimer = null;
    }, 5000);
  }

  function formatBytes(bytes) {
    if (bytes < 1024) return bytes + " B";
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
    if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + " MB";
    return (bytes / (1024 * 1024 * 1024)).toFixed(1) + " GB";
  }

  function escapeHtml(s) {
    var el = document.createElement("span");
    el.textContent = s;
    return el.innerHTML;
  }

  // --- Tab Management ---

  function fetchSessions() {
    fetch("/api/sessions")
      .then(function(r) { return r.json(); })
      .then(function(list) {
        sessions = list;
        renderTabs();
        // If no active session, pick the first one
        if (!activeSessionId && sessions.length > 0) {
          switchToSession(sessions[0].id);
        }
      })
      .catch(function(err) { console.error("Failed to fetch sessions:", err); });
  }

  function renderTabs() {
    tabsEl.innerHTML = "";
    sessions.forEach(function(s) {
      var tab = document.createElement("div");
      tab.className = "tab" + (s.id === activeSessionId ? " active" : "");
      tab.dataset.id = s.id;

      var label = document.createElement("span");
      label.className = "tab-label";
      label.textContent = s.title || s.url || "Tab " + s.id;
      tab.appendChild(label);

      if (sessions.length > 1) {
        var closeBtn = document.createElement("button");
        closeBtn.className = "tab-close";
        closeBtn.innerHTML = "&times;";
        closeBtn.addEventListener("click", function(e) {
          e.stopPropagation();
          closeSession(s.id);
        });
        tab.appendChild(closeBtn);
      }

      tab.addEventListener("click", function() {
        if (activeSessionId !== s.id) {
          switchToSession(s.id);
        }
      });

      tabsEl.appendChild(tab);
    });
  }

  function switchToSession(id) {
    activeSessionId = id;
    renderTabs();
    // Update URL bar with session's current URL
    var s = sessions.find(function(x) { return x.id === id; });
    if (s && s.url) { urlInput.value = s.url; }
    // Reconnect WebSocket to the new session
    lastImage = null;
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    if (ws) { ws.close(); ws = null; }
    connect();
  }

  function createNewSession() {
    fetch("/api/sessions", { method: "POST", headers: {"Content-Type": "application/json"}, body: JSON.stringify({url: "https://www.google.com"}) })
      .then(function(r) { return r.json(); })
      .then(function(s) {
        sessions.push(s);
        switchToSession(s.id);
      })
      .catch(function(err) { console.error("Failed to create session:", err); });
  }

  function closeSession(id) {
    fetch("/api/sessions/" + id, { method: "DELETE" })
      .then(function() {
        sessions = sessions.filter(function(s) { return s.id !== id; });
        if (activeSessionId === id) {
          if (sessions.length > 0) {
            switchToSession(sessions[0].id);
          } else {
            activeSessionId = null;
            renderTabs();
          }
        } else {
          renderTabs();
        }
      })
      .catch(function(err) { console.error("Failed to close session:", err); });
  }

  tabNewBtn.addEventListener("click", function(e) {
    e.stopPropagation();
    createNewSession();
  });

  // --- Service Worker ---

  if ("serviceWorker" in navigator) {
    navigator.serviceWorker.register("/sw.js").catch(function () {});
  }

  // --- Audio Playback ---

  function initAudio() {
    if (audioCtx) return;
    try {
      audioCtx = new (window.AudioContext || window.webkitAudioContext)();
      audioNextTime = 0;
    } catch (e) {
      console.error("Failed to create AudioContext:", e);
    }
  }

  function handleAudioPacket(arrayBuf) {
    if (!audioCtx) return;
    if (audioCtx.state === "suspended") {
      audioCtx.resume();
    }

    var view = new DataView(arrayBuf, 1);
    var sampleRate = view.getUint32(0, true);
    var channels = view.getUint32(4, true);
    var frames = view.getUint32(8, true);

    // Copy PCM data to aligned buffer (offset 13 is not 4-byte aligned)
    var pcmBytes = new Uint8Array(arrayBuf, 13);
    var alignedBuf = new ArrayBuffer(pcmBytes.length);
    new Uint8Array(alignedBuf).set(pcmBytes);
    var pcmData = new Float32Array(alignedBuf);

    var audioBuffer = audioCtx.createBuffer(channels, frames, sampleRate);

    // De-interleave into per-channel arrays
    for (var ch = 0; ch < channels; ch++) {
      var channelData = audioBuffer.getChannelData(ch);
      for (var i = 0; i < frames; i++) {
        channelData[i] = pcmData[i * channels + ch];
      }
    }

    var source = audioCtx.createBufferSource();
    source.buffer = audioBuffer;
    source.connect(audioCtx.destination);

    var currentTime = audioCtx.currentTime;
    if (audioNextTime < currentTime) {
      audioNextTime = currentTime + 0.05;
    }
    source.start(audioNextTime);
    audioNextTime += frames / sampleRate;
  }

  // --- Init ---

  window.addEventListener("resize", resizeCanvas);
  if (window.visualViewport) {
    window.visualViewport.addEventListener("resize", resizeCanvas);
  }
  resizeCanvas();
  fetchSessions();
})();
