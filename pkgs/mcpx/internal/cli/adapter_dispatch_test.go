package cli

import (
	"context"
	"strings"
	"testing"
)

// TestCmdAdapterAddRemoveForwardStrippedArgs guards the dispatch
// contract main and the import branch each assumed: CmdAdapter strips
// the subcommand into rest, and add/remove must receive rest, not the
// original args — otherwise adapter add demo -- git declares an
// adapter named add and adapter remove demo deletes remove.
// (Merge regression caught by Codex review on PR #2647, 2026-10-30.)
func TestCmdAdapterAddRemoveForwardStrippedArgs(t *testing.T) {
	a := &App{}
	if err := a.CmdAdapter(context.Background(), []string{"add"}); err == nil ||
		!strings.Contains(err.Error(), "usage: mcpx adapter add <name>") {
		t.Errorf("adapter add with no name: got %v, want the add usage error (subcommand must not be forwarded as the name)", err)
	}
	if err := a.CmdAdapter(context.Background(), []string{"remove"}); err == nil ||
		!strings.Contains(err.Error(), "usage: mcpx adapter remove <name>") {
		t.Errorf("adapter remove with no name: got %v, want the remove usage error (subcommand must not be forwarded as the name)", err)
	}
}
