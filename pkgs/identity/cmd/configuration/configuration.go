package configuration

import (
	"github.com/developing-today/code/src/identity/configuration"
	"github.com/knadh/koanf"
	"github.com/samber/do/v2"
)

type ConfigurationService interface {
	Configuration() any
	Koanf() (*koanf.Koanf, error)
}

type ConfigurationServiceImpl struct {
	config    *koanf.Koanf
	separator string
	locations *configuration.ConfigurationLocations
}

func (cs *ConfigurationServiceImpl) Configuration() any {
	return cs.config
}

func (cs *ConfigurationServiceImpl) Koanf() (*koanf.Koanf, error) {
	return cs.config, nil
}

func NewConfigurationService(config *koanf.Koanf, separator string, locations *configuration.ConfigurationLocations) func(do.Injector) (ConfigurationService, error) {
	return func(i do.Injector) (ConfigurationService, error) {
		return &ConfigurationServiceImpl{
			config:    config,
			separator: separator,
			locations: locations,
		}, nil
	}
}
