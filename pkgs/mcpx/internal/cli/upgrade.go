package cli

import (
	"context"
	"errors"
	"fmt"
	"os"
)

// CmdUpgrade hands the running daemon over to this binary. It runs from the
// new build, so an activation script does not need the old daemon's path.
// It never starts a daemon: with none running there is nothing to upgrade.
func (a *App) CmdUpgrade(ctx context.Context, args []string) error {
	if err := parseFlags(a, newFlagSet("upgrade"), args); err != nil {
		return err
	}
	paths, _ := a.resolvePathsForConfig()
	c := NewClient(paths, a.ConfigPath)
	if !c.Ping(ctx) {
		return errors.New("no mcpx daemon is running for this configuration, so there is nothing to upgrade")
	}
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	res, err := c.Upgrade(ctx, exe)
	if err != nil {
		return fmt.Errorf("upgrade: %w", err)
	}
	if res["status"] == "current" {
		fmt.Println("already current")
		return nil
	}
	fmt.Printf("upgraded the running daemon to %s\n", exe)
	return nil
}
