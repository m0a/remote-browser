# --- Builder stage ---
FROM rust:1.88-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake ninja-build clang libclang-dev \
    pkg-config libssl-dev \
    libnss3-dev libnspr4-dev libatk1.0-dev libatk-bridge2.0-dev \
    libcups2-dev libdrm-dev libgbm-dev libxkbcommon-dev \
    libasound2-dev libdbus-1-dev \
    libx11-dev libxcomposite-dev libxdamage-dev libxext-dev libxfixes-dev \
    libxrandr-dev libxrender-dev libxtst-dev libxi-dev \
    libpango1.0-dev libcairo2-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY wew/ wew/

# Build release and stage CEF runtime files
RUN cargo build --release 2>&1 \
    && mkdir -p /cef-staging/locales \
    && cp /build/target/release/cef-browser /cef-staging/ \
    && cp /build/target/release/build/wew-*/out/cef/Release/*.so* /cef-staging/ \
    && cp /build/target/release/build/wew-*/out/cef/Release/*.bin /cef-staging/ \
    && cp /build/target/release/build/wew-*/out/cef/Resources/*.pak /cef-staging/ \
    && cp /build/target/release/build/wew-*/out/cef/Resources/*.dat /cef-staging/ \
    && cp /build/target/release/build/wew-*/out/cef/Resources/locales/*.pak /cef-staging/locales/

# --- Runtime stage ---
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    xvfb \
    fonts-noto-cjk \
    libx11-6 libxcomposite1 libxdamage1 libxext6 libxfixes3 \
    libxrandr2 libxrender1 libxtst6 libxi6 \
    libnss3 libnspr4 libatk1.0-0 libatk-bridge2.0-0 \
    libcups2 libdrm2 libgbm1 libpango-1.0-0 libcairo2 \
    libasound2 libdbus-1-3 libxkbcommon0 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary and CEF runtime files from staging
COPY --from=builder /cef-staging/ ./

# Copy static files
COPY public/ ./public/

ENV PORT=3000
ENV PUBLIC_DIR=/app/public
ENV DISPLAY=:99
ENV LD_LIBRARY_PATH=/app

EXPOSE 3000

ENTRYPOINT ["/bin/sh", "-c", "\
    Xvfb :99 -screen 0 1280x720x24 -ac -nolisten tcp & \
    sleep 1 && \
    exec /app/cef-browser \
        --no-sandbox \
        --disable-gpu \
        --disable-gpu-compositing \
        --disable-software-rasterizer \
        --remote-debugging-port=${CDP_PORT:-9222} \
        \"${START_URL:-https://www.google.com}\" \
    "]
