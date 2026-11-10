package daemon

import (
	"fmt"
	"net"
	"net/http"
	"strings"
)

// refuseBrowserPages keeps web pages away from /v1.
//
// The daemon listens on loopback TCP as well as its unix socket, and /v1 is
// unauthenticated: the socket's file permissions are the access control, and
// a TCP port has none. The authorisation vocabulary (#1968) does not change
// that -- it names the caller (a principal per surface, authz.go) and what
// each operation would require, and the check answers yes; this refusal of
// browser pages is a transport defence, not an authentication, and stays
// the only thing standing between a web page and /v1 over TCP. A page in any browser on this machine can send a
// "simple" cross-origin POST -- text/plain, no preflight -- to
// 127.0.0.1:<port>/v1/exec, and the handler decodes the body as JSON
// regardless of its declared type. So every page the user visits could run
// code as the user, limited only by guessing the port. /mcp already refused
// foreign origins, as the MCP specification requires; the /v1 routes beside
// it on the same listener did not.
//
// Two rules, the same ones the MCP transport applies:
//
//   - An Origin header is sent by browsers and by nothing else mcpx talks
//     to. One that is not loopback, not the daemon's own address and not in
//     transport.allowedOrigins is refused.
//   - Over TCP, while the daemon is bound only to loopback, the Host header
//     must name loopback. That is what stops DNS rebinding: a page that has
//     rebound its own name to 127.0.0.1 is same-origin to itself, sends no
//     Origin on a GET, and would otherwise read /v1/log.
//
// A request with no Origin over the socket -- every CLI, script and plugin
// call -- is untouched.
func (s *Server) refuseBrowserPages(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if origin := r.Header.Get("Origin"); origin != "" && !s.Origins.Allows(origin) {
			writeErr(w, http.StatusForbidden, fmt.Errorf("origin %s may not use this daemon; "+
				"add it to transport.allowedOrigins if it should", origin))
			return
		}
		if overTCP(r) && s.loopbackOnly() {
			if ip, ok := tailnetLocalIP(r); ok {
				// This request arrived on the tailnet listener, whose
				// clients name the daemon by its MagicDNS name, not by
				// a loopback name — the loopback rule would refuse
				// every legitimate tsnet caller. The tailnet is an
				// authenticated network, so the Host rule there is
				// the mirror image: the name must be this daemon's
				// own tailnet name or address.
				if !s.tailnetHost(r.Host, ip) {
					writeErr(w, http.StatusForbidden, fmt.Errorf("host %q is not this daemon's tailnet name; "+
						"the tailnet listener answers only to its own name", r.Host))
					return
				}
			} else if !s.loopbackHost(r.Host) {
				writeErr(w, http.StatusForbidden, fmt.Errorf("host %q is not this daemon's; "+
					"a loopback daemon answers only to a loopback name", r.Host))
				return
			}
		}
		next.ServeHTTP(w, r)
	})
}

func overTCP(r *http.Request) bool {
	addr, ok := r.Context().Value(http.LocalAddrContextKey).(net.Addr)
	return ok && addr.Network() == "tcp"
}

// loopbackOnly reports whether the TCP listener is bound to loopback, which
// is the only case where the set of names that reach it is known.
func (s *Server) loopbackOnly() bool {
	if s.Address == "" {
		return true
	}
	ip := net.ParseIP(strings.Trim(s.Address, "[]"))
	return strings.EqualFold(s.Address, "localhost") || (ip != nil && ip.IsLoopback())
}

// tailnetLocalIP returns the local address of a request that arrived on
// a tailnet listener: a TCP address in Tailscale's ranges (100.64.0.0/10
// or the fd7a:115c:a1e0::/48 ULA). ok is false for any other request.
func tailnetLocalIP(r *http.Request) (net.IP, bool) {
	addr, ok := r.Context().Value(http.LocalAddrContextKey).(net.Addr)
	if !ok || addr.Network() != "tcp" {
		return nil, false
	}
	host, _, err := net.SplitHostPort(addr.String())
	if err != nil {
		host = addr.String()
	}
	ip := net.ParseIP(strings.Trim(host, "[]"))
	if ip == nil {
		return nil, false
	}
	if v4 := ip.To4(); v4 != nil {
		return ip, v4[0] == 100 && v4[1]&0xc0 == 0x40
	}
	ula := net.ParseIP("fd7a:115c:a1e0::")
	return ip, ip.Mask(net.CIDRMask(48, 128)).Equal(ula)
}

// tailnetHost reports whether host names this daemon on its tailnet: its
// own tailnet IP, or its configured tsnet hostname (short, or as a
// prefix of the full MagicDNS name a client dialed).
func (s *Server) tailnetHost(host string, localIP net.IP) bool {
	h := host
	if split, _, err := net.SplitHostPort(host); err == nil {
		h = split
	}
	h = strings.TrimSuffix(strings.Trim(h, "[]"), ".")
	if ip := net.ParseIP(h); ip != nil {
		return ip.Equal(localIP)
	}
	name := strings.TrimSuffix(s.set.String("daemon.tsnet.hostname"), ".")
	if name == "" {
		return false
	}
	lower := strings.ToLower(h)
	// The full MagicDNS form is <hostname>.<tailnet>.ts.net; a bare
	// prefix match would also accept mcpx-daemon.evil.example.
	return strings.EqualFold(h, name) ||
		(strings.HasPrefix(lower, strings.ToLower(name)+".") && strings.HasSuffix(lower, ".ts.net"))
}

func (s *Server) loopbackHost(host string) bool {
	h := host
	if split, _, err := net.SplitHostPort(host); err == nil {
		h = split
	}
	h = strings.Trim(h, "[]")
	if strings.EqualFold(h, "localhost") {
		return true
	}
	ip := net.ParseIP(h)
	return ip != nil && ip.IsLoopback()
}
