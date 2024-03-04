package configuration

import (
	"embed"
	"os"
	"strings"
	"syscall"
	"time"

	"github.com/auth0/go-jwt-middleware/v2/jwks"
	"github.com/charmbracelet/log"
	"github.com/developing-today/code/src/identity/configuration/namespace"
	"github.com/knadh/koanf"
	"github.com/knadh/koanf/parsers/kdl"
	"github.com/knadh/koanf/providers/env"
	"github.com/knadh/koanf/providers/file"
	"github.com/knadh/koanf/providers/rawbytes"
)

/*
// todo
embed default kdl file,
default kdl file ->
hard code vars in build ->
config file -> env vars ->
remote config (

	s3 ->
	db ->
	nats/centrifuge ->
	etc) (
	dont do all this, just this is the direction eventually as things become available if)
*/

// todo: put these into configuration but also as flat defaults in configuration
var Separator = "."
var ConfigurationFilePath = "config.kdl"
var EmbeddedConfigurationFilePath = "embed/config.kdl"
var GeneratedKeyDirPath = ".ssh/generated"
var HostKeyPath = ".ssh/term_info_ed25519"
var ScpFileSystemDirPath = "scp"
var DefaultDoneSignals = []os.Signal{os.Interrupt, syscall.SIGINT, syscall.SIGTERM}

//go:embed all:embed
var EmbedFS embed.FS

type ConfigurationLocations struct {
	ConfigurationFilePaths         []string
	EmbeddedConfigurationFilePaths []string
}

type IdentityServerConfiguration struct {
	Configuration          *koanf.Koanf
	ConfigurationLocations *ConfigurationLocations
	ConfigurationSeparator string
	EmbedFS                *embed.FS
	JWKSProvider           *jwks.Provider
	JWTAudience            []string
}

func GetEnv(key string, defaultValue string) string {
	value := os.Getenv(strings.ToUpper(key))
	if value == "" {
		return defaultValue
	}
	return value
}

var (
	Prefix = GetEnv(namespace.Prefix, "dt")
)

func (c *IdentityServerConfiguration) LoadConfiguration() {
	// lower, replace prefix with identity.", also populate "" and "identity.server."
	// IDENTITY_SERVER_*
	// lower, replace prefix with "identity.", also populate ""
	// IDENTITY_*
	// lower, replace prefix with "charm.", also populate "charm.server."
	// CHARM_SERVER_*
	// lower, replace prefix with "charm."
	// CHARM_*

	// example of loading env var
	// Load environment variables and merge into the loaded config.
	// "MYVAR" is the prefix to filter the env vars by.
	// "." is the delimiter used to represent the key hierarchy in env vars.
	// The (optional, or can be nil) function can be used to transform
	// the env var names, for instance, to lowercase them.
	//
	// For example, env vars: MYVAR_TYPE and MYVAR_PARENT1_CHILD1_NAME
	// will be merged into the "type" and the nested "parent1.child1.name"
	// keys in the config file here as we lowercase the key,
	// replace `_` with `.` and strip the MYVAR_ prefix so that
	// only "parent1.child1.name" remains.
	// k.Load(env.Provider("MYVAR_", ".", func(s string) string {
	// 	return strings.Replace(strings.ToLower(
	// 		strings.TrimPrefix(s, "MYVAR_")), "_", ".", -1)
	// }), nil)

	// decide order of loading, defaults, env, file, env, file, remotes, etc.
	// always overwrite previous?
	// ever skip if already set?

	// for file loading:
	// maybe. not sure yet, especially for "".
	// 1. loading from file as ""
	// 2. loading from file as "identity.", put 1 in 2 where not set
	// 3. loading from file as "identity.server.", put 2 in 3 where not set
	// if i set port=1 and identity.server.ssh.port=3, then ""1,"identity"1,"identity.server"3
	// if i set port=1 and identity.port=3, then ""1,"identity"3,"identity.server"3

	// IDENTITY_SERVER_AUTHORIZATION_HEADER_PREFIX_PATH := "identity.server.authorization.header_name"
	// if os.Getenv("IDENTITY_SERVER_AUTHORIZATION_HEADER_NAME") != "" {
	// 	c.Configuration.Set(IDENTITY_SERVER_AUTHORIZATION_HEADER_PREFIX_PATH, os.Getenv("IDENTITY_SERVER_AUTHORIZATION_HEADER_NAME"))
	// }
	// c.Configuration.Set(IDENTITY_SERVER_AUTHORIZATION_HEADER_PREFIX_PATH, "Bearer")

	prefix := os.Getenv("DT_PREFIX") // todo allow overrides
	if prefix == "" {
		prefix = "dt"
	}
	c.Configuration.Set("prefix", prefix)

	c.Configuration.Set("identity.server.authorization.cookie_name", "Authorization")
	c.Configuration.Set("identity.server.authorization.header_name", "Authorization")

	// IDENTITY_SERVER_AUTHORIZATION_HEADER_NAME := os.Getenv("IDENTITY_SERVER_AUTHORIZATION_HEADER_NAME")

	c.Configuration.Set("identity.server.authorization.header_prefix", "Bearer")
	c.Configuration.Set("identity.server.host", "0.0.0.0")
	c.Configuration.Set("identity.server.jwt.audience", "identity")
	c.Configuration.Set("identity.server.jwt.cache_ttl", time.Duration(15)*time.Minute)
	c.Configuration.Set("identity.server.port", 1)
	c.Configuration.Set("identity.server.web.port", 7000)

	c.Configuration.Set("identity.server.nats.host", "localhost")
	c.Configuration.Set("identity.server.nats.port", 4222)
	c.Configuration.Set("identity.server.tls.cert.server", "./data/tls/cert.pem")
	c.Configuration.Set("identity.server.tls.cert.client", "./data/tls/client-cert.pem")
	c.Configuration.Set("identity.server.tls.key.server", "./data/tls/key.pem")
	c.Configuration.Set("identity.server.tls.key.client", "./data/tls/client-key.pem")
	c.Configuration.Set("identity.server.tls.ca", "./data/tls/ca.pem")

	log.Info("Loaded default configuration", "config", c.Configuration.Sprint())
	log.Info("Loading embedded file configuration", "config", c.ConfigurationLocations.EmbeddedConfigurationFilePaths)

	for _, path := range c.ConfigurationLocations.EmbeddedConfigurationFilePaths {
		data, err := c.EmbedFS.ReadFile(path)
		if err != nil {
			log.Info("Embedded Config not found or error reading", "path", path, "error", err)
			continue
		}

		if err := c.Configuration.Load(rawbytes.Provider(data), kdl.Parser()); err != nil {
			log.Error("Failed to load embedded config", "error", err)
		} else {
			log.Info("Loaded config from embedded file", "path", path)
		}
	}

	log.Info("Loaded embedded configuration", "config", c.Configuration.Sprint())
	log.Info("Loading environment configuration", "environment_variable_prefix", prefix, "lvl", "WARN")

	c.Configuration.Load(env.Provider(prefix, ".", func(s string) string {
		return strings.Replace(strings.Replace(strings.Replace(strings.ToLower(
			strings.TrimPrefix(s, prefix)),
			"__", " ", -1),
			"_", ".", -1),
			" ", "_", -1)
	}), nil)

	log.Info("Loaded environment configuration", "config", c.Configuration.Sprint())
	log.Info("Loading file configuration", "paths", c.ConfigurationLocations.ConfigurationFilePaths)

	for _, path := range c.ConfigurationLocations.ConfigurationFilePaths {
		if _, err := os.Stat(path); err == nil {
			if err := c.Configuration.Load(file.Provider(path), kdl.Parser()); err != nil {
				log.Error("Failed to load file config", "error", err)
			} else {
				log.Info("Loaded config from file", "path", path)
			}
		} else {
			log.Info("Config file not found", "path", path)
		}
	}

	log.Info("Loaded file configuration", "config", c.Configuration.Sprint())
}

func (c *IdentityServerConfiguration) SetConfiguration(config *IdentityServerConfiguration) {
	c.Configuration = config.Configuration
	c.ConfigurationLocations = config.ConfigurationLocations
	c.EmbedFS = config.EmbedFS
	c.JWTAudience = config.JWTAudience
	c.JWKSProvider = config.JWKSProvider
}
