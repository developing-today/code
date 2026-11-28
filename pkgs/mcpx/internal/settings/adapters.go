package settings

// adapterSettings are the knobs for adapter declarations: the files
// paths.adapters names that turn command-line programs into servers.

func adapterSettings() []Setting {
	return []Setting{
		{
			Path: "adapters.allowBare", Kind: KindBool, Default: "false",
			Scope: ScopeClient,
			Name:  "Allow bare adapters",
			Short: "permit bare (passthrough) adapter declarations to load",
			Long: "A bare declaration exposes a binary's whole argv surface as one " +
				"passthrough tool: nothing in the declaration constrains what a " +
				"caller can reach (#1962). That is a deliberate thing to write by " +
				"hand and a dangerous thing to inherit from a file somebody else " +
				"wrote, so loading one is refused unless this setting is on. " +
				"Off (the default), any declaration with \"bare\": true fails to " +
				"load with an error naming this setting; on, bare declarations " +
				"load exactly as any other. An organisation can therefore forbid " +
				"passthrough declarations centrally by leaving it off (#11147).",
		},
	}
}
