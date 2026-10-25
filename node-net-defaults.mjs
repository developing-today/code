// node-net-defaults.mjs - per-process Node network defaults, loaded via
// NODE_OPTIONS=--import by commandcode-proxy-start.sh (laptop only).
//
// Why: this LAN's IPv6 is ULA-only with no upstream - the router advertises
// fd2e:2810:acf5::/48 plus a default route but answers
// Destination-unreachable for global IPv6 (tracepath !N at hop 1, router
// NDP entry FAILED). Node 24's global fetch (undici) with the default
// autoSelectFamily=true times out against it (ETIMEDOUT, all hosts), while
// plain net.connect / https.get succeed. Turning autoSelectFamily OFF makes
// undici use plain sequential connects in getaddrinfo order (IPv4 first on
// this host), which work.
// Scope: only Node processes whose env carries this NODE_OPTIONS - i.e. the
// commandcode-proxy tree (proxy + ACP CLI children, which inherit env).
// Undo: remove the NODE_OPTIONS export from commandcode-proxy-start.sh.
import net from 'node:net';

try {
  net.setDefaultAutoSelectFamily(false);
} catch {
  // Node without this API: leave defaults untouched.
}
