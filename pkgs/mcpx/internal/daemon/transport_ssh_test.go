package daemon

import "testing"

func TestParseSSHEndpoint(t *testing.T) {
	cases := []struct {
		endpoint string
		want     SSHEndpoint
		ok       bool
		wantErr  bool
	}{
		{endpoint: "ssh://buildbox", want: SSHEndpoint{Host: "buildbox", Bin: SSHDefaultBin}, ok: true},
		{endpoint: "ssh://drew@buildbox:2222", want: SSHEndpoint{User: "drew", Host: "buildbox", Port: 2222, Bin: SSHDefaultBin}, ok: true},
		{endpoint: "ssh://buildbox?bin=/opt/mcpx/bin/mcpx", want: SSHEndpoint{Host: "buildbox", Bin: "/opt/mcpx/bin/mcpx"}, ok: true},
		{endpoint: "ssh://buildbox?sock=/run/mcpx/daemon.sock", want: SSHEndpoint{Host: "buildbox", Bin: SSHDefaultBin, Sock: "/run/mcpx/daemon.sock"}, ok: true},
		{endpoint: "ssh://buildbox/", want: SSHEndpoint{Host: "buildbox", Bin: SSHDefaultBin}, ok: true},

		{endpoint: "ssh://", ok: true, wantErr: true},
		{endpoint: "ssh://buildbox:notaport", ok: true, wantErr: true},
		{endpoint: "ssh://buildbox:70000", ok: true, wantErr: true},
		{endpoint: "ssh://buildbox/path", ok: true, wantErr: true},
		{endpoint: "ssh://user:pw@buildbox", ok: true, wantErr: true},
		{endpoint: "ssh://buildbox?bogus=1", ok: true, wantErr: true},

		{endpoint: "tsnet://laptop.tailnet.ts.net", ok: false},
		{endpoint: "unix:///tmp/mcpx.sock", ok: false},
		{endpoint: "http://127.0.0.1:8065", ok: false},
		{endpoint: "", ok: false},
	}
	for _, tc := range cases {
		got, ok, err := ParseSSHEndpoint(tc.endpoint)
		if ok != tc.ok || (err != nil) != tc.wantErr {
			t.Errorf("ParseSSHEndpoint(%q) ok=%v err=%v; want ok=%v err=%v", tc.endpoint, ok, err, tc.ok, tc.wantErr)
			continue
		}
		if tc.wantErr || !ok {
			continue
		}
		if got != tc.want {
			t.Errorf("ParseSSHEndpoint(%q) = %+v; want %+v", tc.endpoint, got, tc.want)
		}
	}
}

// A parse round trip: what String prints must parse back to the same
// endpoint, so an error message quoting the endpoint still describes the
// route the user can type again.
func TestSSHEndpointStringRoundTrip(t *testing.T) {
	eps := []SSHEndpoint{
		{Host: "buildbox", Bin: SSHDefaultBin},
		{User: "drew", Host: "buildbox", Port: 2222, Bin: SSHDefaultBin},
		{Host: "buildbox", Bin: "/opt/mcpx", Sock: "/run/mcpx/daemon.sock"},
	}
	for _, ep := range eps {
		back, ok, err := ParseSSHEndpoint(ep.String())
		if !ok || err != nil || back != ep {
			t.Errorf("%+v printed %q, parsed back %+v ok=%v err=%v", ep, ep.String(), back, ok, err)
		}
	}
}

// The remote command is what runs on the far side: it names the remote
// mcpx, the proxy subcommand, and only ever an explicit socket.
func TestSSHEndpointRemoteCommand(t *testing.T) {
	if got := (SSHEndpoint{Host: "h"}).RemoteCommand(); len(got) != 2 || got[0] != "mcpx" || got[1] != SSHProxyCommand {
		t.Errorf("RemoteCommand() = %v", got)
	}
	got := SSHEndpoint{Host: "h", Bin: "/opt/mcpx", Sock: "/run/d.sock"}.RemoteCommand()
	want := []string{"/opt/mcpx", SSHProxyCommand, "--sock", "/run/d.sock"}
	if len(got) != len(want) {
		t.Fatalf("RemoteCommand() = %v; want %v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("RemoteCommand() = %v; want %v", got, want)
		}
	}
}
