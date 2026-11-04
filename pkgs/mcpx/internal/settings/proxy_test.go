package settings

import (
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

func TestProxyTimeoutUsesBuiltinDefault(t *testing.T) {
	var timeout *Setting
	for _, setting := range Registry() {
		if setting.Path == "proxy.timeout" {
			timeout = &setting
			break
		}
	}
	if timeout == nil {
		t.Fatal("proxy.timeout is not registered")
	}
	if timeout.Kind != KindDuration {
		t.Errorf("proxy.timeout kind = %q, want duration", timeout.Kind)
	}
	if timeout.Default != defaults.Builtin().Proxy.Timeout {
		t.Errorf("proxy.timeout default = %q, want defaults.json value %q", timeout.Default, defaults.Builtin().Proxy.Timeout)
	}
}

func TestProxyTimeoutMustBePositive(t *testing.T) {
	setting := proxySettings()[0]
	for _, raw := range []string{"0s", "-1s"} {
		if err := Validate(setting, raw); err == nil {
			t.Errorf("Validate(proxy.timeout, %q) succeeded, want an error", raw)
		}
	}
}
