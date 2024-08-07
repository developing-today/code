package all

import (
	"context"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/charmbracelet/log"
	"github.com/developing-today/code/src/identity/cmd/charm"
	"github.com/developing-today/code/src/identity/cmd/command"
	icfg "github.com/developing-today/code/src/identity/cmd/configuration"
	contextservice "github.com/developing-today/code/src/identity/cmd/context"
	d "github.com/developing-today/code/src/identity/cmd/do"
	ssh "github.com/developing-today/code/src/identity/cmd/ssh"
	idc "github.com/developing-today/code/src/identity/cmd/ssh/configuration"
	"github.com/developing-today/code/src/identity/cmd/stream"
	"github.com/developing-today/code/src/identity/configuration"
	"github.com/samber/do/v2"
	"github.com/spf13/cobra"
)

func StartAllAltCmd(command cobra.Command) *cobra.Command {
	result := command
	result.Use = "all"
	result.Aliases = []string{"al", "a"}
	return &result
}

func StartAllCmd(ctx context.Context, config *configuration.SshServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	result := &cobra.Command{
		Use:     "start",
		Short:   "Starts the identity and charm servers",
		Run:     StartAllServices(ctx, config),
		Aliases: []string{"s", "run", "serve", "publish", "pub", "p", "i", "y", "u", "o", "p", "q", "w", "e", "r", "t", "a", "s", "d", "f", "g", "h", "j", "k", "l", "z", "x", "c", "v", "b"},
	}
	result.AddCommand(charm.StartCharmCmd(ctx, config), ssh.StartSshCmd(ctx, config), stream.StartStreamCmd(ctx, config))
	result.AddCommand(StartAllAltCmd(*result))
	return result
}

func StartAllServices(ctx context.Context, config *configuration.SshServerConfiguration) func(*cobra.Command, []string) {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	return func(cmd *cobra.Command, args []string) {
		if ctx == nil {
			ctx = context.Background()
		}
		StartServices(ctx, config)(cmd, args)
	}
}

func StartServices(ctx context.Context, config *configuration.SshServerConfiguration) func(*cobra.Command, []string) {
	return func(cmd *cobra.Command, args []string) {
		log.Info("Setting up shutdown context")
		ctx, cancel := context.WithCancel(ctx)
		defer FinalShutdown(ctx, cmd, args)
		defer cancel()

		osDone := make(chan os.Signal, 1)
		defer signal.Stop(osDone)
		doneSignals := configuration.DefaultDoneSignals
		signal.Notify(osDone, doneSignals...)

		iDone := make(chan os.Signal, 1)
		defer signal.Stop(iDone)

		done := make(chan os.Signal, 1)
		defer signal.Stop(done)

		go func() {
			log.Info("Waiting for signals to shutdown services")
			select {
			case <-osDone:
				log.Info("Signal received, shutting down services")
			case <-ctx.Done():
				log.Info("Context cancelled, ensuring all services complete.")
			}
			log.Info("Canceling context")
			cancel()
			log.Info("Cancelled context. waiting for services to complete.")
		}()
		go func() {
			log.Info("Waiting for signals injector has shutdown")
			<-iDone
			log.Info("Signal received, injector has shutdown")
			cancel()
			log.Info("Cancelled context. Sending final shutdown signal")
			done <- syscall.SIGINT
			log.Info("Final shutdown signal sent")
		}()

		log.Info("Creating injector")

		i := do.NewWithOpts(&do.InjectorOpts{
			HookAfterRegistration: func(scope *do.Scope, serviceName string) {
				log.Warn("Registered service", "serviceName", serviceName)
			},
			HookAfterShutdown: func(scope *do.Scope, serviceName string) {
				log.Warn("Shutdown service", "serviceName", serviceName)
			},
			Logf: func(format string, args ...interface{}) {
				if format == "DI <scope=[root]>: requested healthcheck" ||
					format == "DI <scope=[root]>: requested healthcheck for service %s" ||
					format == "DI <scope=[root]>: got healthcheck results: %v" {
					log.Debugf(format, args...)
					return
				}
				log.Warnf(format, args...)
			},
		})

		log.Info("Injector created, setting up injector shutdown signal")

		go func() {
			select {
			case <-ctx.Done():
				log.Info("Context done, shutting down")

				errors := i.Shutdown()
				log.Info("Shutdown complete")

				if errors != nil {
					for _, err := range *errors {
						log.Error("Error shutting down", "error", err)
					}
				} else {
					log.Info("All services have been shut down")
				}
				iDone <- syscall.SIGINT
			}
		}()
		go func() {
			log.Info("Waiting for signals to shutdown injector")
			signal, errors := i.ShutdownOnSignals(doneSignals...)
			log.Info("Signals received, injector was shutdown", "signal", signal, "errors", errors)
			if errors != nil {
				for _, err := range *errors {
					log.Error("Error shutting down", "error", err)
				}
			}
			log.Info("Signals received, shutdown")
			osDone <- signal
			iDone <- signal
		}()

		log.Info("Providing services")

		d.Provide(i, contextservice.NewContextService(ctx))
		d.Provide(i, idc.NewSshServerConfigurationService(config))
		d.Provide(i, command.NewCommandService(cmd, args))
		d.Provide(i, icfg.NewConfigurationService(config.Configuration, config.ConfigurationSeparator, config.ConfigurationLocations))
		d.Provide(i, charm.NewCharmService)
		d.Provide(i, ssh.NewSshService)
		d.Provide(i, stream.NewStreamService)
		d.Provide(i, ssh.NewStreamClientService)

		log.Info("Starting services")
		d.Start[charm.CharmService](i)
		d.Start[ssh.SshService](i)

		log.Info("Services started")

		go func() {
			time.Sleep(5 * time.Second)
			for {
				health := i.HealthCheck()
				for k, v := range health {
					if v != nil {
						log.Error("Health check failed", "service", k, "error", v)
						cancel()
					}
				}
				time.Sleep(15 * time.Second)
			}
		}()

		log.Info("Waiting for signals to shutdown")
		<-done
	}
}

func FinalShutdown(ctx context.Context, cmd *cobra.Command, args []string) {
	if ctx == nil {
		ctx = context.Background()
	}
	if cmd == nil {
		panic("cmd is nil")
	}
	log.Info("All services cleaned up. Shutting down.", "command", cmd.Name(), "args", args)
	log.Info("Bye!", "time", time.Now())
}
