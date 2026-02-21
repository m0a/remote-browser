(function () {
  "use strict";

  var canvas = document.getElementById("screen");
  var ctx = canvas.getContext("2d");
  var statusEl = document.getElementById("status");
  var kbToggle = document.getElementById("kb-toggle");
  var hiddenInput = document.getElementById("hidden-input");
  var urlBar = document.getElementById("url-bar");

  var ws = null;
  var metadata = null;
  var frameRect = { x: 0, y: 0, width: 0, height: 0 };
  var lastImage = null;
  var connected = false;
  var reconnectTimer = null;

  // --- Canvas Setup ---

  function resizeCanvas() {
    var dpr = window.devicePixelRatio || 1;
    canvas.width = window.innerWidth * dpr;
    canvas.height = window.innerHeight * dpr;
    canvas.style.width = window.innerWidth + "px";
    canvas.style.height = window.innerHeight + "px";
    if (lastImage) drawFrame(lastImage);
  }

  // --- Frame Drawing ---

  function drawFrame(img) {
    lastImage = img;
    var cw = canvas.width;
    var ch = canvas.height;
    var iw = img.naturalWidth || img.width;
    var ih = img.naturalHeight || img.height;

    var scale = Math.min(cw / iw, ch / ih);
    var dw = iw * scale;
    var dh = ih * scale;
    var dx = (cw - dw) / 2;
    var dy = (ch - dh) / 2;

    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, cw, ch);
    ctx.drawImage(img, dx, dy, dw, dh);

    frameRect = { x: dx, y: dy, width: dw, height: dh };
  }

  // --- Coordinate Mapping ---

  function clientToCDP(clientX, clientY) {
    if (!metadata) return null;

    var rect = canvas.getBoundingClientRect();
    var dpr = window.devicePixelRatio || 1;

    // Canvas pixel coordinates
    var cx = (clientX - rect.left) * dpr;
    var cy = (clientY - rect.top) * dpr;

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

    ws.onmessage = function (e) {
      try {
        var msg = JSON.parse(e.data);
        if (msg.type === "frame") {
          metadata = msg.metadata;
          var img = new Image();
          img.onload = function () {
            drawFrame(img);
          };
          img.src = "data:image/jpeg;base64," + msg.data;
        } else if (msg.type === "url") {
          urlBar.textContent = msg.url;
          urlBar.classList.remove("hidden");
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

  // --- Touch Events ---

  function handleTouch(e, eventType) {
    e.preventDefault();
    var touchPoints = [];
    var touches = e.touches;
    for (var i = 0; i < touches.length; i++) {
      var coords = clientToCDP(touches[i].clientX, touches[i].clientY);
      if (coords) {
        touchPoints.push({
          x: coords.x,
          y: coords.y,
          id: touches[i].identifier,
          radiusX: touches[i].radiusX || 1,
          radiusY: touches[i].radiusY || 1,
          force: touches[i].force || 1,
        });
      }
    }
    send({
      type: "input_touch",
      eventType: eventType,
      touchPoints: touchPoints,
    });
  }

  canvas.addEventListener(
    "touchstart",
    function (e) {
      handleTouch(e, "touchStart");
    },
    { passive: false }
  );
  canvas.addEventListener(
    "touchmove",
    function (e) {
      handleTouch(e, "touchMove");
    },
    { passive: false }
  );
  canvas.addEventListener(
    "touchend",
    function (e) {
      handleTouch(e, "touchEnd");
    },
    { passive: false }
  );
  canvas.addEventListener(
    "touchcancel",
    function (e) {
      handleTouch(e, "touchCancel");
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

  // Text input from soft keyboard
  hiddenInput.addEventListener("input", function (e) {
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
    if (document.activeElement === hiddenInput) return;
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
    if (document.activeElement === hiddenInput) return;
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

  // --- Service Worker ---

  if ("serviceWorker" in navigator) {
    navigator.serviceWorker.register("/sw.js").catch(function () {});
  }

  // --- Init ---

  window.addEventListener("resize", resizeCanvas);
  resizeCanvas();
  connect();
})();
