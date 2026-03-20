const CACHE_NAME = "remote-browser-v11";
const SHELL_FILES = [
  "/",
  "/index.html",
  "/app.js",
  "/style.css",
  "/manifest.json",
  "/icon.svg",
];

self.addEventListener("install", (e) => {
  e.waitUntil(
    caches.open(CACHE_NAME).then((cache) => cache.addAll(SHELL_FILES))
  );
  self.skipWaiting();
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys.filter((k) => k !== CACHE_NAME).map((k) => caches.delete(k))
        )
      )
  );
  self.clients.claim();
});

self.addEventListener("fetch", (e) => {
  // Don't cache WebSocket or API requests
  if (e.request.url.includes("/ws") || e.request.url.includes("/api/")) return;

  // Network first, fallback to cache
  e.respondWith(fetch(e.request).catch(() => caches.match(e.request)));
});
