package auth

import (
	"github.com/charmbracelet/log"
	"github.com/charmbracelet/ssh"
)

func Middleware(h ssh.Handler) ssh.Handler {
	return func(s ssh.Session) {
		log.Info("Session started", "session", s, "sessionID", s.Context().SessionID(), "user", s.Context().User(), "remoteAddr", s.Context().RemoteAddr().String(), "remoteAddrNetwork", s.Context().RemoteAddr().Network(), "localAddr", s.Context().LocalAddr().String(), "localAddrNetwork", s.Context().LocalAddr().Network(), "charm-id", s.Context().Permissions().Extensions["charm-id"], "charm-name", s.Context().Permissions().Extensions["charm-name"], "charm-roles", s.Context().Permissions().Extensions["charm-roles"], "charm-created-at", s.Context().Permissions().Extensions["charm-created-at"], "charm-public-key-created-at", s.Context().Permissions().Extensions["charm-public-key-created-at"], "charm-public-key-type", s.Context().Permissions().Extensions["charm-public-key-type"], "charm-public-key", s.Context().Permissions().Extensions["charm-public-key"])
		h(s)
		log.Info("Session ended", "session", s, "sessionID", s.Context().SessionID(), "user", s.Context().User(), "remoteAddr", s.Context().RemoteAddr().String(), "remoteAddrNetwork", s.Context().RemoteAddr().Network(), "localAddr", s.Context().LocalAddr().String(), "localAddrNetwork", s.Context().LocalAddr().Network(), "charm-id", s.Context().Permissions().Extensions["charm-id"], "charm-name", s.Context().Permissions().Extensions["charm-name"], "charm-roles", s.Context().Permissions().Extensions["charm-roles"], "charm-created-at", s.Context().Permissions().Extensions["charm-created-at"], "charm-public-key-created-at", s.Context().Permissions().Extensions["charm-public-key-created-at"], "charm-public-key-type", s.Context().Permissions().Extensions["charm-public-key-type"], "charm-public-key", s.Context().Permissions().Extensions["charm-public-key"])
	}
}
