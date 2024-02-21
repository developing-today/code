package observability

import (
	"github.com/charmbracelet/log"
	"github.com/charmbracelet/ssh"
	"github.com/developing-today/code/src/identity/auth"
)

// todo need to also handle cron/long-running tasks for cleanup
func Middleware(connections *auth.SafeConnectionMap) func(next ssh.Handler) ssh.Handler {
	return func(next ssh.Handler) ssh.Handler {
		return func(s ssh.Session) {
			connectionId := s.Context().Permissions().Extensions["connection-id"]
			log.Info("Session started", "session", s, "sessionID", s.Context().SessionID(), "user", s.Context().User(), "remoteAddr", s.Context().RemoteAddr().String(), "remoteAddrNetwork", s.Context().RemoteAddr().Network(), "localAddr", s.Context().LocalAddr().String(), "localAddrNetwork", s.Context().LocalAddr().Network(), "charm-id", s.Context().Permissions().Extensions["charm-id"], "charm-name", s.Context().Permissions().Extensions["charm-name"], "charm-roles", s.Context().Permissions().Extensions["charm-roles"], "charm-created-at", s.Context().Permissions().Extensions["charm-created-at"], "charm-public-key-created-at", s.Context().Permissions().Extensions["charm-public-key-created-at"], "charm-public-key-type", s.Context().Permissions().Extensions["charm-public-key-type"], "charm-public-key", s.Context().Permissions().Extensions["charm-public-key"], "connection-id", connectionId)
			conn, ok := connections.Get(connectionId)
			if !ok {
				panic("connection not found")
			}
			next(s)
			status := "closed"
			conn.Status = &status
			connections.Set(s.Context().Permissions().Extensions["connection-id"], &conn)
			log.Info("Session ended", "session", s, "sessionID", s.Context().SessionID(), "user", s.Context().User(), "remoteAddr", s.Context().RemoteAddr().String(), "remoteAddrNetwork", s.Context().RemoteAddr().Network(), "localAddr", s.Context().LocalAddr().String(), "localAddrNetwork", s.Context().LocalAddr().Network(), "charm-id", s.Context().Permissions().Extensions["charm-id"], "charm-name", s.Context().Permissions().Extensions["charm-name"], "charm-roles", s.Context().Permissions().Extensions["charm-roles"], "charm-created-at", s.Context().Permissions().Extensions["charm-created-at"], "charm-public-key-created-at", s.Context().Permissions().Extensions["charm-public-key-created-at"], "charm-public-key-type", s.Context().Permissions().Extensions["charm-public-key-type"], "charm-public-key", s.Context().Permissions().Extensions["charm-public-key"], "connection-id", connectionId)
		}
	}
}
