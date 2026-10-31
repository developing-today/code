package config_test

import (
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/config"
)

// #1649: two enabled servers resolving to the same namespace must be
// refused by ResolveAll, naming both servers and the shared namespace —
// otherwise the generated client declares the namespace twice and no
// script can run.

func collisionErr(t *testing.T, c *config.Config) string {
	t.Helper()
	_, err := c.ResolveAll()
	if err == nil {
		t.Fatal("a namespace collision must be refused")
	}
	return err.Error()
}

func TestResolveAllRejectsSanitizedCollision(t *testing.T) {
	c := &config.Config{MCPServers: map[string]*config.Server{
		"my-server": {Name: "my-server", Command: "x"},
		"my_server": {Name: "my_server", Command: "y"},
	}}
	msg := collisionErr(t, c)
	for _, want := range []string{`servers "my-server" and "my_server"`, `namespace "my_server"`} {
		if !strings.Contains(msg, want) {
			t.Errorf("error should contain %q: %v", want, msg)
		}
	}
}

func TestResolveAllRejectsReservedSuffixCollision(t *testing.T) {
	// "log" is a reserved word, so SafeNamespace suffixes it to "log_",
	// which a server literally named "log_" already owns.
	c := &config.Config{MCPServers: map[string]*config.Server{
		"log":  {Name: "log", Command: "x"},
		"log_": {Name: "log_", Command: "y"},
	}}
	msg := collisionErr(t, c)
	if !strings.Contains(msg, `namespace "log_"`) {
		t.Errorf("error should name the shared namespace: %v", msg)
	}
}

func TestResolveAllRejectsExplicitCollision(t *testing.T) {
	c := &config.Config{MCPServers: map[string]*config.Server{
		"a": {Name: "a", Command: "x", Mcpx: &config.Extras{Namespace: "shared_ns"}},
		"b": {Name: "b", Command: "y", Mcpx: &config.Extras{Namespace: "shared_ns"}},
	}}
	msg := collisionErr(t, c)
	for _, want := range []string{`servers "a" and "b"`, `namespace "shared_ns"`} {
		if !strings.Contains(msg, want) {
			t.Errorf("error should contain %q: %v", want, msg)
		}
	}
}

func TestResolveAllRejectsExplicitVsSanitizedCollision(t *testing.T) {
	c := &config.Config{MCPServers: map[string]*config.Server{
		"gamma":     {Name: "gamma", Command: "x", Mcpx: &config.Extras{Namespace: "my_server"}},
		"my-server": {Name: "my-server", Command: "y"},
	}}
	msg := collisionErr(t, c)
	if !strings.Contains(msg, `namespace "my_server"`) {
		t.Errorf("error should name the shared namespace: %v", msg)
	}
}

func TestResolveAllRejectsAliasCollision(t *testing.T) {
	// An alias keeps its own namespace; "my_server" aliasing "target"
	// still collides with the plain server "my-server".
	c := &config.Config{MCPServers: map[string]*config.Server{
		"target":    {Name: "target", Command: "x"},
		"my-server": {Name: "my-server", Command: "y"},
		"my_server": {Name: "my_server", AliasOf: "target"},
	}}
	msg := collisionErr(t, c)
	if !strings.Contains(msg, `namespace "my_server"`) {
		t.Errorf("error should name the shared namespace: %v", msg)
	}
}

func TestResolveAllDisabledCollisionPasses(t *testing.T) {
	c := &config.Config{MCPServers: map[string]*config.Server{
		"my-server": {Name: "my-server", Command: "x"},
		"my_server": {Name: "my_server", Command: "y", Mcpx: &config.Extras{Disabled: true}},
	}}
	all, err := c.ResolveAll()
	if err != nil {
		t.Fatalf("a disabled server must not collide: %v", err)
	}
	if len(all) != 1 || all[0].Namespace != "my_server" {
		t.Fatalf("expected only the enabled server: %+v", all)
	}
}

func TestResolveAllDistinctNamespacesPassUnchanged(t *testing.T) {
	c := &config.Config{MCPServers: map[string]*config.Server{
		"alpha": {Name: "alpha", Command: "x"},
		"beta":  {Name: "beta", Command: "y"},
	}}
	all, err := c.ResolveAll()
	if err != nil {
		t.Fatal(err)
	}
	if len(all) != 2 || all[0].Namespace != "alpha" || all[1].Namespace != "beta" {
		t.Fatalf("non-colliding namespaces must be unchanged: %+v", all)
	}
}
