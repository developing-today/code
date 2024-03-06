package root

import (
	"context"

	charmcmd "github.com/charmbracelet/charm/cmd"

	"github.com/developing-today/code/src/identity/cmd/all"
	"github.com/developing-today/code/src/identity/cmd/ssh/configuration"
	cfg "github.com/developing-today/code/src/identity/configuration"
	"github.com/spf13/cobra"
)

func DefaultRootCmd() *cobra.Command {
	return RootCmd(context.Background(), configuration.LoadDefaultConfiguration())
}

func DefaultRootCmdWithContext(ctx context.Context) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	return RootCmd(ctx, configuration.LoadDefaultConfiguration())
}

func RootCmd(ctx context.Context, config *cfg.SshServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	result := &cobra.Command{
		Use:   "identity",
		Short: "publish your identity",
		Long:  `publish your identity and allow others to connect to you.`,
	}
	result.AddCommand(charmcmd.RootCmd, all.StartAllCmd(ctx, config))
	return result
}
