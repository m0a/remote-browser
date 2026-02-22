# --- Builder stage ---
FROM rust:1.86-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake ninja-build clang libclang-dev \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY wew/ wew/

# Build release
RUN cargo build --release 2>&1

# --- Runtime stage ---
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    xvfb \
    fonts-noto-cjk \
    libx11-6 libxcomposite1 libxdamage1 libxext6 libxfixes3 \
    libxrandr2 libxrender1 libxtst6 libxi6 \
    libnss3 libnspr4 libatk1.0-0 libatk-bridge2.0-0 \
    libcups2 libdrm2 libgbm1 libpango-1.0-0 libcairo2 \
    libasound2 libdbus-1-3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary
COPY --from=builder /build/target/release/cef-browser ./

# Copy CEF runtime files
COPY --from=builder /build/target/release/build/wew-*/out/cef/Release/*.so* ./
COPY --from=builder /build/target/release/build/wew-*/out/cef/Release/*.bin ./
COPY --from=builder /build/target/release/build/wew-*/out/cef/Release/*.dat ./
COPY --from=builder /build/target/release/build/wew-*/out/cef/Resources/*.pak ./
COPY --from=builder /build/target/release/build/wew-*/out/cef/Resources/*.dat ./
COPY --from=builder /build/target/release/build/wew-*/out/cef/Resources/locales/ ./locales/

# Copy static files
COPY public/ ./public/

ENV PORT=3000
ENV PUBLIC_DIR=/app/public
ENV DISPLAY=:99

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
