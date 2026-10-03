// Reverse proxy for singer.wlta.cc -> the kroak-time-ticker ngrok tunnel.
//
// Three jobs:
//   1. Front the ngrok free-tier hostname with our own domain, so the
//      hostname visitors see doesn't change if the ngrok tunnel is ever
//      recreated with a different name (only UPSTREAM_HOST here needs to
//      change).
//   2. Inject `ngrok-skip-browser-warning` on every request, so visitors
//      (including OBS Browser Source, which can't set custom headers) never
//      hit ngrok's free-tier interstitial warning page.
//   3. When the host is unreachable (its internet is down, the ngrok agent
//      isn't running, or it's up but the local kroak-time-ticker isn't),
//      ngrok's own edge still answers -- with its branded error page, not
//      ours. Detect that and redirect to wlta.cc/offline instead, so
//      viewers (often an unattended OBS Browser Source) see something
//      sensible rather than ngrok's page. That page lives in the
//      wlta-website repo (not duplicated here) so there's one copy to keep
//      current.
//
// Update UPSTREAM_HOST and redeploy (`npm run deploy`) whenever the ngrok
// tunnel's assigned domain changes.
const UPSTREAM_HOST = "cortex-prompter-refract.ngrok-free.dev";
const OFFLINE_PAGE_URL = "https://wlta.cc/offline/";

// ngrok's error pages (tunnel offline, local service unreachable, etc. --
// ERR_NGROK_3200, ERR_NGROK_8012, ...) are always served from body
// id="ngrok" with an ERR_NGROK_xxxx code, regardless of which failure it
// is or what HTTP status it's wrapped in. That marker is distinctive enough
// that the real app would never produce it, so it's safe to match on
// without pinning to one status code.
async function isNgrokErrorPage(response) {
  const contentType = response.headers.get("content-type") || "";
  if (!contentType.includes("text/html")) return false;
  const body = await response.clone().text();
  return body.includes('id="ngrok"') && /ERR_NGROK_\d+/.test(body);
}

function redirectToOfflinePage() {
  // 302, not 301: this is a transient condition (the tunnel comes back),
  // and a 301 risks getting cached past that. no-store on top for browsers
  // that cache redirects more aggressively than the status alone implies.
  return new Response(null, {
    status: 302,
    headers: {
      location: OFFLINE_PAGE_URL,
      "cache-control": "no-store",
    },
  });
}

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

    let response;
    try {
      response = await fetch(proxyRequest);
    } catch {
      // Couldn't even reach ngrok's edge -- treat it the same as ngrok
      // reporting the tunnel offline.
      return redirectToOfflinePage();
    }

    if (await isNgrokErrorPage(response)) {
      return redirectToOfflinePage();
    }
    return response;
  },
};
