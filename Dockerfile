FROM ghcr.io/m1k1o/neko/google-chrome:latest

USER root

# socat: CDP proxy (Chrome binds DevTools to 127.0.0.1 only)
RUN apt-get update && \
    apt-get install -y --no-install-recommends socat && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/* /var/cache/apt/*

# supervisord config (Chrome + socat proxy + openbox)
COPY supervisord.conf /etc/neko/supervisord/google-chrome.conf

# Enable DevTools (default policy disables it)
COPY policies.json /etc/opt/chrome/policies/managed/policies.json

# Window manager config
COPY openbox.xml /etc/neko/openbox.xml

# Chrome profile directory
RUN mkdir -p /home/neko/chrome-profile && \
    chown -R neko:neko /home/neko/chrome-profile

# CDP proxy port
EXPOSE 9223
