package charm

import (
	"context"
	"os"
	"syscall"

	"github.com/developing-today/code/src/identity/cmd/command"
	ctx "github.com/developing-today/code/src/identity/cmd/context"

	charmcmd "github.com/charmbracelet/charm/cmd"
	"github.com/charmbracelet/log"
	"github.com/developing-today/code/src/identity/configuration"
	"github.com/samber/do/v2"
	"github.com/spf13/cobra"
)

type CharmService interface {
	Start()
	Shutdown() error
	HealthCheck() error
	IsCharmService() bool
}

type CharmServiceImpl struct {
	command    command.CommandService
	context    ctx.ContextService
	cancelFunc context.CancelFunc
	ctx        context.Context
}

func (cs *CharmServiceImpl) IsCharmService() bool {
	return true
}

func NewCharmService(i do.Injector) (*CharmServiceImpl, error) {
	service := &CharmServiceImpl{
		context: ctx.MustGetContextService(i),
		command: command.MustGetCommandService(i),
	}
	go service.Start()
	return service, nil
}

func (cs *CharmServiceImpl) Start() {
	go func() {
		log.Info("Starting charm server")
		if cs.ctx != nil {
			panic("ctx is already set, service is already running")
		}
		cs.ctx, cs.cancelFunc = context.WithCancel(cs.context.Context())
		if err := charmcmd.ServeCmdRunEWithContext(cs.ctx, cs.command.GetCommand(), cs.command.GetArgs()); err != nil {
			log.Error("Error running charm server command", "error", err)
			panic(err)
		}
	}()
}

func (cs *CharmServiceImpl) Shutdown() error {
	log.Info("Charm service shutdown requested")
	if cs.cancelFunc != nil && cs.ctx.Err() == nil {
		log.Info("Charm service stopping...")
		cs.cancelFunc()
		cs.context.Shutdown() // this service shuts down the parent context
		log.Info("Charm service stopped")
	} else if cs.ctx.Err() != nil {
		log.Info("Charm service already stopped")
	} else {
		log.Info("Charm service has not been started")
	}
	return nil
}

func WrappedCharmFromContext(ctx context.Context, config *configuration.SshServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	cmd := charmcmd.RootCmd

	go func() {
		<-ctx.Done()

		p, err := os.FindProcess(os.Getpid()) // TODO: fix this remove it or something, this doesn't work on windows
		if err != nil {
			log.Error("could not find process", "error", err)
			return
		}
		if err := p.Signal(syscall.SIGINT); err != nil {
			log.Error("could not send interrupt signal", "error", err)
		}
	}()

	return cmd
}

func CharmCmd(ctx context.Context, config *configuration.SshServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	return WrappedCharmFromContext(ctx, config)
}

func StartCharmCmd(ctx context.Context, config *configuration.SshServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	result := WrappedCharmFromContext(ctx, config)
	result.Use = "charm"
	result.Aliases = []string{"ch", "c"}
	return result
}

// HealthCheck performs a health check of the charm service.
func (cs *CharmServiceImpl) HealthCheck() error {
	// Placeholder for health check logic
	// log.Info("Health check passed for CharmService")
	return nil
}
