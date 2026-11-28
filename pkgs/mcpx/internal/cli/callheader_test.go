package cli

import (
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// #1864 item 4: pool.callTimeout is daemon-scoped, but a value the user
// actually gave must still ride the call-settings header as a request
// (the daemon clamps it to its configured ceiling). The default must
// not ride: an unset knob is not a request.
func TestCallHeaderCarriesPoolCallTimeoutRequest(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	if h := callHeader(settings.NewSet(sch)); strings.Contains(h, "pool.callTimeout") {
		t.Fatalf("default set sent a pool.callTimeout request: %s", h)
	}
	given := settings.NewSet(sch).WithOverrides(map[string]string{"pool.callTimeout": "5s"}, "test")
	h := callHeader(given)
	if !strings.Contains(h, `"pool.callTimeout":"5s"`) {
		t.Fatalf("given value missing from call header: %s", h)
	}
}
