// Reverse proxy for singer.wlta.cc -> the kroak-time-ticker ngrok tunnel.
//
// Two jobs:
//   1. Front the ngrok free-tier hostname with our own domain, so the
//      hostname visitors see doesn't change if the ngrok tunnel is ever
//      recreated with a different name (only UPSTREAM_HOST here needs to
//      change).
//   2. Inject `ngrok-skip-browser-warning` on every request, so visitors
//      (including OBS Browser Source, which can't set custom headers) never
//      hit ngrok's free-tier interstitial warning page.
//
// Update UPSTREAM_HOST and redeploy (`npm run deploy`) whenever the ngrok
// tunnel's assigned domain changes.
const UPSTREAM_HOST = "cortex-prompter-refract.ngrok-free.dev";

export default {
  async fetch(request) {
    const url = new URL(request.url);
    url.protocol = "https:";
    url.hostname = UPSTREAM_HOST;
    url.port = "";

    const headers = new Headers(request.headers);
    headers.set("ngrok-skip-browser-warning", "true");

    const proxyRequest = new Request(url, {
      method: request.method,
      headers,
      body: request.body,
    });

    return fetch(proxyRequest);
  },
};
