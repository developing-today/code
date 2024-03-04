package command

import (
	d "github.com/developing-today/code/src/identity/cmd/do"
	"github.com/samber/do/v2"
	"github.com/spf13/cobra"
)

type CommandService interface {
	GetCommand() *cobra.Command
	GetArgs() []string
}

type CommandServiceImpl struct {
	cmd  *cobra.Command
	args []string
}

func (cs *CommandServiceImpl) GetCommand() *cobra.Command {
	return cs.cmd
}

func (cs *CommandServiceImpl) GetArgs() []string {
	return cs.args
}

func NewCommandService(cmd *cobra.Command, args []string) func(do.Injector) (CommandService, error) {
	return func(i do.Injector) (CommandService, error) {
		return &CommandServiceImpl{
			cmd:  cmd,
			args: args,
		}, nil
	}
}

func MustGetCommandService(i do.Injector) CommandService {
	return d.MustInvokeAny[CommandService](i)
}
