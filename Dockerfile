FROM oven/bun:1-debian AS base

# Chrome + Xvfb + 日本語フォント + websockify/noVNC
RUN apt-get update && apt-get install -y --no-install-recommends \
    wget gnupg ca-certificates \
    xvfb socat x11vnc \
    novnc websockify \
    fonts-noto-cjk fonts-noto-cjk-extra \
    && wget -q -O - https://dl.google.com/linux/linux_signing_key.pub | gpg --dearmor -o /usr/share/keyrings/google-chrome.gpg \
    && echo "deb [arch=amd64 signed-by=/usr/share/keyrings/google-chrome.gpg] http://dl.google.com/linux/chrome/deb/ stable main" > /etc/apt/sources.list.d/google-chrome.list \
    && apt-get update && apt-get install -y --no-install-recommends google-chrome-stable \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# ソースコピー
COPY viewer/ ./viewer/

# ポート: viewer HTTP/WS + CDP proxy + noVNC
EXPOSE 3000 9223 6080

# Chrome プロファイル
VOLUME ["/app/chrome-profile"]

CMD ["bun", "run", "viewer/server.ts"]
