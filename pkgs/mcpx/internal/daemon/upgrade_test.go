package daemon

import (
	"context"
	"encoding/json"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/config"
)

func startUpgradeServer(t *testing.T) (*Server, *http.Client) {
	t.Helper()
	dir, err := os.MkdirTemp("", "mxh")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	paths := Paths{State: dir, Cache: dir, Socket: filepath.Join(dir, "d.sock"), Info: filepath.Join(dir, "d.json")}
	srv, err := NewServer(Options{
		Config:  &config.Config{MCPServers: map[string]*config.Server{}},
		Paths:   paths,
		Version: "test",
		Logger:  log.New(io.Discard, "", 0),
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(srv.reg.Close)
	if err := srv.Listen(0); err != nil {
		t.Fatal(err)
	}
	srv.startHTTP()
	t.Cleanup(func() { _ = srv.httpSrv.Close() })
	client := &http.Client{Transport: &http.Transport{
		DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			var d net.Dialer
			return d.DialContext(ctx, "unix", paths.Socket)
		},
	}}
	return srv, client
}

func postUpgrade(t *testing.T, client *http.Client, binary string) (int, map[string]any) {
	t.Helper()
	body, err := json.Marshal(map[string]string{"binary": binary})
	if err != nil {
		t.Fatal(err)
	}
	resp, err := client.Post("http://mcpx"+upgradeRoute, "application/json", strings.NewReader(string(body)))
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	var out map[string]any
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		t.Fatal(err)
	}
	return resp.StatusCode, out
}

func assertHealthy(t *testing.T, client *http.Client) {
	t.Helper()
	resp, err := client.Get("http://mcpx/v1/health")
	if err != nil {
		t.Fatalf("the daemon stopped answering: %v", err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("health: %s", resp.Status)
	}
}

func TestUpgradeToTheRunningBinaryIsCurrent(t *testing.T) {
	_, client := startUpgradeServer(t)
	self, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	code, out := postUpgrade(t, client, self)
	if code != http.StatusOK || out["status"] != "current" {
		t.Fatalf("upgrade to the running binary: %d %v", code, out)
	}
}

func TestUpgradeWithAMissingBinaryKeepsServing(t *testing.T) {
	_, client := startUpgradeServer(t)
	missing := filepath.Join(t.TempDir(), "no-such-mcpx")
	code, out := postUpgrade(t, client, missing)
	msg, _ := out["error"].(string)
	if code != http.StatusInternalServerError || !strings.Contains(msg, "start "+missing) {
		t.Fatalf("upgrade to a missing binary: %d %v", code, out)
	}
	assertHealthy(t, client)
}

func TestUpgradeSuccessorInheritsNotifySocketAndDiesBeforeTakeover(t *testing.T) {
	dir := t.TempDir()
	notify := filepath.Join(dir, "notify.sock")
	t.Setenv("NOTIFY_SOCKET", notify)
	seen := filepath.Join(dir, "seen")
	next := filepath.Join(dir, "next")
	script := "#!/bin/sh\nprintf '%s' \"$NOTIFY_SOCKET\" > " + seen + "\nexit 3\n"
	if err := os.WriteFile(next, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}

	srv, client := startUpgradeServer(t)
	code, out := postUpgrade(t, client, next)
	msg, _ := out["error"].(string)
	if code != http.StatusInternalServerError || !strings.Contains(msg, "exited before it took over") {
		t.Fatalf("upgrade to a successor that exits: %d %v", code, out)
	}
	got, err := os.ReadFile(seen)
	if err != nil {
		t.Fatalf("the successor did not run: %v", err)
	}
	if string(got) != notify {
		t.Fatalf("the successor saw NOTIFY_SOCKET %q, want %q", got, notify)
	}
	deadline := time.Now().Add(5 * time.Second)
	for srv.upgrading.Load() != nil {
		if time.Now().After(deadline) {
			t.Fatal("the upgrade stayed registered after it failed")
		}
		time.Sleep(10 * time.Millisecond)
	}
	assertHealthy(t, client)
}
