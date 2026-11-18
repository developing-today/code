package daemon

import (
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// daemon.tsnetUpTimeout was declared and never read: both the daemon's
// listener and the client's dialer waited the built-in default whatever
// the setting said. The budget now comes from the setting, and only an
// unset one falls back.
func TestTSNetUpBudgetHonoursTheSetting(t *testing.T) {
	if got := tsnetUpBudget(0); got != defaults.TSNetUpTimeout {
		t.Fatalf("an unset budget should be the default %s, got %s", defaults.TSNetUpTimeout, got)
	}
	if got := tsnetUpBudget(90 * time.Second); got != 90*time.Second {
		t.Fatalf("a configured budget should be used as is, got %s", got)
	}
	d := NewTSNetDialer("mcpx-client", 45*time.Second)
	if d.upTimeout != 45*time.Second {
		t.Fatalf("the dialer should carry the configured budget, got %s", d.upTimeout)
	}
}
