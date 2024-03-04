package context

import (
	"context"

	d "github.com/developing-today/code/src/identity/cmd/do"

	"github.com/charmbracelet/log"
	"github.com/samber/do/v2"
)

type ContextService interface {
	Shutdown() error
	HealthCheck() error
	Context() context.Context
	CancelFunc() context.CancelFunc
	IsContextService() bool
}

func MustGetNewContext(i do.Injector) (context.Context, context.CancelFunc) {
	return context.WithCancel(d.MustInvokeAny[ContextService](i).Context())
}

func MustGetContextService(i do.Injector) ContextService {
	return d.MustInvokeAny[ContextService](i)
}

type ContextServiceImpl struct {
	ctx        context.Context
	cancelFunc context.CancelFunc
}

func (is *ContextServiceImpl) IsContextService() bool {
	return true
}

func (is *ContextServiceImpl) HealthCheck() error {
	if is.ctx.Err() != nil {
		log.Error("Health check failed for ContextService", "error", is.ctx.Err())
		return is.ctx.Err()
	}
	log.Info("Health check passed for ContextService")
	return nil
}

func (is *ContextServiceImpl) CancelFunc() context.CancelFunc {
	return is.cancelFunc
}

func (is *ContextServiceImpl) Context() context.Context {
	return is.ctx
}

func NewContextService(ctx context.Context) func(do.Injector) (ContextService, error) {
	if ctx == nil {
		ctx = context.Background()
	}
	ctx, cancelFunc := context.WithCancel(ctx)
	return NewContextServiceWithCancel(ctx, cancelFunc)
}

func NewContextServiceWithCancel(ctx context.Context, cancelFunc context.CancelFunc) func(do.Injector) (ContextService, error) {
	return func(i do.Injector) (ContextService, error) {
		service := &ContextServiceImpl{
			ctx:        ctx,
			cancelFunc: cancelFunc,
		}
		return service, nil
	}
}

func (is *ContextServiceImpl) Shutdown() error {
	log.Info("Context service shutdown requested")
	if is.cancelFunc != nil && is.ctx.Err() == nil {
		log.Info("Context service stopping...")
		is.cancelFunc()
		log.Info("Context service stopped")
	} else if is.ctx.Err() != nil {
		log.Info("Context service already stopped")
	} else {
		log.Info("Context service has not been started")
	}
	return nil
}
