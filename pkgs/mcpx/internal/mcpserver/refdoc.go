package mcpserver

import "context"

// refdocNotifier is a Notifier that delivers nothing. It exists so the
// reference documentation can evaluate the server's declared
// capabilities for a connection that can push, without a client.
type refdocNotifier struct{}

func (refdocNotifier) Listen(context.Context, ListenFilter, func(string, any)) {}

// DeclaredServerCapabilities is the capability map mcpx declares for one
// protocol revision on a connection that can push (#4909, remainder of
// #1846 slice 4).
//
// capabilities() is deliberately connection-shaped — "can push" is a
// property of the connection, not the server — so a document cannot
// read it directly. This evaluates it the way a push-capable stdio
// connection would see it: a notifier is installed and the connection
// has a send function. Push-dependent entries in the result therefore
// mean "declared when the connection can push", which is exactly the
// condition the protocol document's §2.2 footnote states.
//
// One value is not a plain yes even here: tools.listChanged also needs
// pass-through mode (mcpx's own tool list is fixed when the server is
// built), and this server is not in pass-through mode, so the entry
// reads false. The document renders that distinction rather than
// hiding it.
func DeclaredServerCapabilities(version string) map[string]any {
	s := New(nil, "mcpx", "")
	s.Notify = refdocNotifier{}
	c := s.newConn("refdoc", func(any) error { return nil })
	return s.capabilities(context.Background(), version, c)
}
