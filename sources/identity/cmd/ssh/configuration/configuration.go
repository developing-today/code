package configuration

import (
	"github.com/developing-today/code/src/identity/configuration"
	"github.com/knadh/koanf"
	"github.com/samber/do/v2"
)

func NewConfiguration() *configuration.SshServerConfiguration {
	return &configuration.SshServerConfiguration{
		ConfigurationSeparator: configuration.Separator,
		Configuration:          koanf.New(configuration.Separator),
		ConfigurationLocations: &configuration.ConfigurationLocations{
			ConfigurationFilePaths: []string{
				configuration.ConfigurationFilePath,
				// identity.kdl identity.config.kdl config.identity.kdl identity.config
				// run these against ? binary dir ? pwd of execution ? appdata ? .config ? .local ???
				// then check for further locations/env-prefixes/etc from first pass, rerun on top with second pass
				// (maybe config.kdl next to binary sets a new set of configurationPaths, finish out loading from defaults, then load from new paths)
				// this pattern continues, after hard-code default env/file search, then custom file/env search, then eventually maybe nats/centrifuge/s3 or other remote or db config
			},
			EmbeddedConfigurationFilePaths: []string{
				configuration.EmbeddedConfigurationFilePath,
			},
		},
		EmbedFS: &configuration.EmbedFS,
	}
}

func LoadDefaultConfiguration() *configuration.SshServerConfiguration {
	config := NewConfiguration()
	config.LoadConfiguration()
	// log.Info("Loaded config", "config", config.Configuration.Sprint())
	return config
}

type SshServiceConfiguration interface {
	Configuration() *configuration.SshServerConfiguration
}

type IdentityServiceConfigurationImpl struct {
	config *configuration.SshServerConfiguration
}

func (isc *IdentityServiceConfigurationImpl) Configuration() *configuration.SshServerConfiguration {
	return isc.config
}

func NewSshServerConfigurationService(config *configuration.SshServerConfiguration) func(do.Injector) (SshServiceConfiguration, error) {
	return func(i do.Injector) (SshServiceConfiguration, error) {
		return &IdentityServiceConfigurationImpl{
			config: config,
		}, nil
	}
}
