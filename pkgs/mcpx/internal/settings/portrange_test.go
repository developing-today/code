package settings_test

import (
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

func TestACeilingRefusesAWholeNumberOutsideItsRange(t *testing.T) {
	// A declared ceiling declares the whole range: the floor (zero when
	// unset) is a real lower bound too, so -1 is refused as firmly as
	// one past the top.
	s := settings.Setting{Path: "x.port", Kind: settings.KindInt, Default: "0", Ceiling: 65535}
	for _, raw := range []string{"-1", "-8080", "65536", "99999"} {
		err := settings.Validate(s, raw)
		if err == nil {
			t.Errorf("%q: want a refusal outside 0-65535", raw)
			continue
		}
		if !strings.Contains(err.Error(), "x.port") || !strings.Contains(err.Error(), raw) {
			t.Errorf("%q: the refusal must name the setting and the value, got %v", raw, err)
		}
	}
	for _, raw := range []string{"0", "8080", "65535"} {
		if err := settings.Validate(s, raw); err != nil {
			t.Errorf("%q: want acceptance inside 0-65535, got %v", raw, err)
		}
	}
}

func TestNoCeilingLeavesAnIntegerAlone(t *testing.T) {
	// Only a setting that declares a ceiling has one. A floor on its
	// own does not invent an upper bound, and an unbounded integer
	// keeps the behaviour it had, large values included.
	s := settings.Setting{Path: "x.n", Kind: settings.KindInt, Default: "1", Floor: 1}
	for _, raw := range []string{"1", "99999"} {
		if err := settings.Validate(s, raw); err != nil {
			t.Errorf("%q: a setting without a ceiling should accept it: %v", raw, err)
		}
	}
}

func TestDaemonPortAcceptsOnlyTCPPorts(t *testing.T) {
	// #3648: daemon.port validated at every layer while net.Listen
	// refused the value, so an impossible port travelled to daemon
	// startup before anything named the setting. NormalizePath is the
	// shared choke point the layers validate through.
	for _, raw := range []string{"0", "8080", "65535"} {
		if _, err := settings.NormalizePath("daemon.port", raw); err != nil {
			t.Errorf("daemon.port %q: want acceptance (0 still means pick a free port), got %v", raw, err)
		}
	}
	for _, raw := range []string{"-1", "65536", "99999"} {
		_, err := settings.NormalizePath("daemon.port", raw)
		if err == nil {
			t.Errorf("daemon.port %q: want a refusal at the settings layer", raw)
			continue
		}
		if !strings.Contains(err.Error(), "daemon.port") || !strings.Contains(err.Error(), raw) {
			t.Errorf("daemon.port %q: the refusal must name the setting and the value, got %v", raw, err)
		}
	}
}
