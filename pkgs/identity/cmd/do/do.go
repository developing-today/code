package do

import (
	"github.com/charmbracelet/log"
	"github.com/samber/do/v2"
	converter "github.com/samber/go-type-to-string"
)

func InvokeAs[T any](i do.Injector) (T, error) {
	t, err := do.InvokeAs[T](i)
	if err == nil {
		log.Info("Invoked as", "type", converter.GetType[T]())
	}
	return t, err
}

func Invoke[T any](i do.Injector) (T, error) {
	t, err := do.Invoke[T](i)
	if err == nil {
		log.Info("Invoked", "type", converter.GetType[T]())
	}
	return t, err
}

func MustInvokeAny[T any](i do.Injector) T {
	t, err1 := Invoke[T](i)
	if err1 == nil {
		return t
	}
	t, err2 := InvokeAs[T](i)
	if err2 == nil {
		return t
	}
	log.Error("Failed to invoke any", "mustInvokeError", err1, "mustInvokeAsError", err2)
	panic(err2)
}

// The first matching service in the scope tree is returned.
func Start[T any](i do.Injector) {
	MustInvokeAny[T](i)
}

// type Provider[T any] func(do.Injector) (T, error)
func Provide[T any](i do.Injector, providers ...do.Provider[T]) {
	name := converter.GetType[T]()
	log.Info("Providing service", "serviceName", name)
	for _, provider := range providers {
		do.Provide[T](i, provider)
		log.Info("Provided service", "serviceName", name)
	}
}
