package settings_test

import (
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

func TestModelProfilesAreRegisteredThroughOneSettingsSchema(t *testing.T) {
	s := schema(t)
	for _, path := range []string{"model.providers", "model.profiles"} {
		setting, ok := s.Lookup(path)
		if !ok {
			t.Fatalf("%s is not in the settings registry", path)
		}
		if setting.Kind != settings.KindString || setting.Default != "{}" || setting.Scope != settings.ScopeDaemon {
			t.Errorf("%s = kind %v default %q scope %v", path, setting.Kind, setting.Default, setting.Scope)
		}
		if err := settings.Validate(*setting, setting.Default); err != nil {
			t.Errorf("%s default invalid: %v", path, err)
		}
	}
	for _, path := range []string{"model.provider", "model.baseURL", "model.apiKeyEnv", "model.default"} {
		if _, ok := s.Lookup(path); !ok {
			t.Errorf("existing singular setting %s disappeared", path)
		}
	}
}
