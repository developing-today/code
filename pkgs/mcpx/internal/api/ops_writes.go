package api

// writesOps exposes the explicit operator action that clears a server after
// an upstream write's outcome became unknown.
func writesOps() []Op {
	return []Op{
		{
			Name:       "writes_clear",
			Method:     "POST",
			Path:       "/v1/writes/{server}/clear",
			Permission: PermissionAdmin,
			Admin:      true,
			Mutating:   true,
			Summary:    "Clear a server's unknown-write block",
			Description: "Allows non-read-only calls to a server again after an operator has checked " +
				"whether a write with an unknown outcome took effect.",
			Params: []Param{
				{Name: "server", In: InPath, Type: "string", Required: true,
					Desc: "the configured server name shown in the refusal"},
			},
		},
	}
}
