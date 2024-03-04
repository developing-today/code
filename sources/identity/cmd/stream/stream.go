package stream

import (
	"context"

	"github.com/charmbracelet/log"
	"github.com/developing-today/code/src/identity/cmd/command"
	ctx "github.com/developing-today/code/src/identity/cmd/context"
	"github.com/developing-today/code/src/identity/configuration"
	"github.com/developing-today/code/src/identity/stream"
	"github.com/samber/do/v2"
	"github.com/spf13/cobra"
)

func StartStreamCmd(ctx context.Context, config *configuration.IdentityServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	return &cobra.Command{
		Use:     "stream",
		Short:   "Starts only the stream server",
		Run:     StartStreamFromContext(ctx),
		Aliases: []string{"tr", "t"},
	}
}

type StreamService interface {
	Start()
	Shutdown() error
	HealthCheck() error
	IsStreamService() bool
}

type StreamServiceImpl struct {
	ctx        context.Context
	cancelFunc context.CancelFunc
	command    command.CommandService
	context    ctx.ContextService
}

func (ss *StreamServiceImpl) IsStreamService() bool {
	return true
}

func (ss *StreamServiceImpl) HealthCheck() error {
	// log.Info("Health check passed for StreamService")
	return nil
}

func NewStreamService(i do.Injector) (StreamService, error) {
	contextService := ctx.MustGetContextService(i)
	command := command.MustGetCommandService(i)
	service := &StreamServiceImpl{
		context: contextService,
		command: command,
	}
	service.Start()
	return service, nil
}

func (ss *StreamServiceImpl) Start() {
	log.Info("Starting stream server")
	if ss.ctx != nil {
		panic("ctx is already set, service is already running")
	}
	ss.ctx, ss.cancelFunc = context.WithCancel(ss.context.Context())
	go stream.RunStreamServer(ss.ctx, ss.command.GetCommand(), ss.command.GetArgs())
}

func (ss *StreamServiceImpl) Shutdown() error {
	log.Info("Stream service shutdown requested")
	if ss.cancelFunc != nil && ss.ctx.Err() == nil {
		log.Info("Stream service stopping...")
		ss.cancelFunc()
		ss.context.Shutdown() // this service shuts down the parent context
		log.Info("Stream service stopped")
	} else {
		log.Info("Stream service already stopped")
	}
	return nil
}

func StartStreamFromContext(ctx context.Context) func(*cobra.Command, []string) {
	if ctx == nil {
		ctx = context.Background()
	}
	return func(cmd *cobra.Command, args []string) {
		StartStream()(ctx)(cmd, args)
	}
}

func StartStream() func(context.Context) func(*cobra.Command, []string) {
	return func(ctx context.Context) func(*cobra.Command, []string) {
		return func(cmd *cobra.Command, args []string) {
			log.Info("Starting stream server")
			if ctx == nil {
				ctx = context.Background()
			}
			stream.RunStreamServer(ctx, cmd, args)
		}
	}
}
