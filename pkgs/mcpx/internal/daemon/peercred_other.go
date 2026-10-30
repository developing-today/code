//go:build !linux

package daemon

import "net"

// peerUID reports no uid on platforms this build does not read peer
// credentials for. The principal falls back to the surface alone
// (api.UnixPrincipal): the socket's file permissions remain the access
// control there, as they were before the vocabulary existed.
func peerUID(*net.UnixConn) (uint32, bool) { return 0, false }
