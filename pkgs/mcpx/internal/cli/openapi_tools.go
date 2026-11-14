package cli

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
	"github.com/developing-today/code/pkgs/mcpx/internal/openapi"
)

// APISpec declares an OpenAPI document as a source of tools.
type APISpec struct {
	Name              string            `json:"name"`
	Spec              string            `json:"spec"`
	BaseURL           string            `json:"baseUrl,omitempty"`
	Include           []string          `json:"include,omitempty"`
	Exclude           []string          `json:"exclude,omitempty"`
	Methods           []string          `json:"methods,omitempty"`
	IncludeDeprecated bool              `json:"includeDeprecated,omitempty"`
	Headers           map[string]string `json:"headers,omitempty"`
	Description       string            `json:"description,omitempty"`
}

// APIFile is a document of them.
type APIFile struct {
	APIs []APISpec `json:"apis"`
}

// loadAPIs reads declarations from paths.apis.
func (a *App) loadAPIs() ([]APISpec, error) {
	return loadAPIFiles(splitPathList(a.Settings().String("paths.apis")))
}

// loadAPIFiles reads and validates declarations from these files.
func loadAPIFiles(paths []string) ([]APISpec, error) {
	var out []APISpec
	for _, p := range paths {
		if p == "" {
			continue
		}
		b, err := os.ReadFile(p)
		if err != nil {
			return nil, fmt.Errorf("reading API declarations from %s: %w", p, err)
		}
		var doc APIFile
		if err := json.Unmarshal(config.StripJSONC(b), &doc); err != nil {
			var bare []APISpec
			if json.Unmarshal(config.StripJSONC(b), &bare) != nil {
				return nil, fmt.Errorf("%s is not a valid API file: %w", p, err)
			}
			doc.APIs = bare
		}
		out = append(out, doc.APIs...)
	}
	return out, nil
}

// buildAPI turns one declaration into callable tools.
func (a *App) buildAPI(ctx context.Context, s APISpec) (*openapi.API, error) {
	spec, err := openapi.Load(ctx, s.Spec, 0)
	if err != nil {
		return nil, err
	}
	// Environment expansion in headers, so a declaration can be checked in
	// and a token cannot. ${VAR} rather than $VAR because a header value
	// legitimately contains dollar signs.
	headers := map[string]string{}
	for k, v := range s.Headers {
		headers[k] = os.Expand(v, func(name string) string { return os.Getenv(name) })
	}
	return openapi.Build(spec, openapi.Options{
		Name: s.Name, BaseURL: s.BaseURL,
		Include: s.Include, Exclude: s.Exclude, Methods: s.Methods,
		IncludeDeprecated: s.IncludeDeprecated, Headers: headers,
	})
}

// CmdAPI inspects and calls OpenAPI-described services.
func (a *App) CmdAPI(ctx context.Context, args []string) error {
	sub := ""
	if len(args) > 0 && !strings.HasPrefix(args[0], "-") {
		sub, args = args[0], args[1:]
	}
	// add and remove go through the daemon, like adapter add/remove: the
	// write has to land where the running daemon reads, and the local
	// declaration files are not the authority for a live change.
	if sub == "add" {
		return a.apiAdd(ctx, args)
	}
	if sub == "remove" {
		return a.apiRemove(ctx, args)
	}
	args = hoistFlags(args, map[string]bool{
		"spec": true, "base-url": true, "name": true,
		"include": true, "exclude": true, "methods": true, "limit": true,
	})
	fs := newFlagSet("api")
	specFlag := fs.String("spec", "", "an OpenAPI document: a path or a URL")
	baseURL := fs.String("base-url", "", "override the server URL the document declares")
	name := fs.String("name", "", "namespace for the generated tools")
	include := fs.String("include", "", "only operations matching these words, comma separated")
	exclude := fs.String("exclude", "", "drop operations matching these words")
	methods := fs.String("methods", "", "HTTP methods to expose; 'all' for every one (default: read-only)")
	deprecated := fs.Bool("deprecated", false, "include operations the document marks deprecated")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}

	// An inline --spec is a declaration of one, so the two paths converge
	// immediately rather than being maintained separately.
	var specs []APISpec
	if *specFlag != "" {
		specs = []APISpec{{
			Name: *name, Spec: *specFlag, BaseURL: *baseURL,
			Include: splitCSV(*include), Exclude: splitCSV(*exclude),
			Methods: splitCSV(*methods), IncludeDeprecated: *deprecated,
		}}
	} else if sub == "serve" && len(fs.Args()) > 1 {
		// Explicit declaration files select the declarations, full stop:
		// the daemon passes them precisely so the child's own discovered
		// configuration (which may name other, missing or invalid
		// paths.apis files) cannot prevent the requested API from
		// serving. Loading the defaults first would fail before the
		// override is ever reached (Codex review, PR #2514).
		loaded, err := loadAPIFiles(fs.Args()[1:])
		if err != nil {
			return err
		}
		specs = loaded
	} else {
		declared, err := a.loadAPIs()
		if err != nil {
			return err
		}
		specs = declared
	}

	switch sub {
	case "", "list":
		if len(specs) == 0 {
			fmt.Println("No API specifications declared.\n" +
				"Point paths.apis at a file, or pass --spec <path-or-url>:\n" +
				"  mcpx api tools --spec https://example.com/openapi.json")
			return nil
		}
		for _, s := range specs {
			api, err := a.buildAPI(ctx, s)
			if err != nil {
				fmt.Printf("%-16s error: %v\n", s.Name, err)
				continue
			}
			fmt.Printf("%-16s %-44s %d operations\n", api.Name, api.BaseURL, len(api.Tools))
		}
		return nil

	case "tools":
		var out []mcpserver.Tool
		for _, s := range specs {
			api, err := a.buildAPI(ctx, s)
			if err != nil {
				return err
			}
			for _, t := range api.Tools {
				out = append(out, mcpserver.Tool{
					Name: t.Name, Description: t.Description, InputSchema: t.InputSchema,
				})
			}
		}
		if a.JSON {
			return a.out(out)
		}
		for _, t := range out {
			fmt.Printf("%s\n    %s\n", t.Name, t.Description)
		}
		fmt.Printf("\n%d operations.\n", len(out))
		return nil

	case "call":
		if fs.NArg() == 0 {
			return fmt.Errorf("usage: mcpx api call <tool> '<json>' [--spec <doc>]")
		}
		tool := fs.Arg(0)
		var payload map[string]any
		if fs.NArg() > 1 {
			if err := json.Unmarshal([]byte(fs.Arg(1)), &payload); err != nil {
				return fmt.Errorf("arguments must be a JSON object: %w", err)
			}
		}
		for _, s := range specs {
			api, err := a.buildAPI(ctx, s)
			if err != nil {
				continue
			}
			for _, t := range api.Tools {
				if t.Name != tool {
					continue
				}
				res, err := api.Call(ctx, tool, payload)
				if err != nil {
					return err
				}
				if a.JSON {
					return a.out(res)
				}
				fmt.Print(res.Body)
				if res.Status >= 400 {
					return fmt.Errorf("%s returned %d", tool, res.Status)
				}
				return nil
			}
		}
		return fmt.Errorf("no operation named %q; `mcpx api tools` lists them", tool)

	case "serve":
		// One declared API as a stdio MCP server. This is what the daemon
		// spawns for every declared API (see augmentAPIs), so an
		// OpenAPI-described service is an upstream server like any other.
		// Files after the name replace paths.apis, so the daemon can say
		// exactly which declarations it meant whatever the child's
		// environment or working directory.
		if len(fs.Args()) < 1 {
			return fmt.Errorf("usage: mcpx api serve <name> [file...]")
		}
		for _, sp := range specs {
			if sp.Name != fs.Arg(0) {
				continue
			}
			srv := mcpserver.New(nil, sp.Name, a.Version)
			srv.ExtrasOnly = true
			if sp.Description != "" {
				srv.OwnInstructions = sp.Description
			}
			extras := a.apiExtras(ctx, []APISpec{sp})
			if len(extras) == 0 {
				return fmt.Errorf("api %q produced no tools; `mcpx api tools` shows why", sp.Name)
			}
			srv = srv.WithExtras(extras)
			return srv.ServeStdio(ctx, os.Stdin, os.Stdout)
		}
		return fmt.Errorf("no api named %q", fs.Arg(0))
	}
	return fmt.Errorf("no api subcommand %q; list, tools, call, serve, add or remove", sub)
}

// apiTools exposes declared APIs through the MCP server.
func (a *App) apiTools(ctx context.Context) []mcpserver.Extra {
	specs, err := a.loadAPIs()
	if err != nil || len(specs) == 0 {
		return nil
	}
	return a.apiExtras(ctx, specs)
}

// apiExtras builds the MCP extras for these API declarations.
func (a *App) apiExtras(ctx context.Context, specs []APISpec) []mcpserver.Extra {
	var out []mcpserver.Extra
	for _, s := range specs {
		api, err := a.buildAPI(ctx, s)
		if err != nil {
			continue
		}
		bound := api
		for _, t := range bound.Tools {
			t := t
			out = append(out, mcpserver.Extra{
				Tool: mcpserver.Tool{
					Name: t.Name, Description: t.Description, InputSchema: t.InputSchema,
				},
				Call: func(ctx context.Context, raw json.RawMessage) (string, error) {
					var args map[string]any
					if len(raw) > 0 {
						if err := json.Unmarshal(raw, &args); err != nil {
							return "", err
						}
					}
					res, err := bound.Call(ctx, t.Name, args)
					if err != nil {
						return "", err
					}
					if res.Status >= 400 {
						return fmt.Sprintf("HTTP %d\n\n%s", res.Status, res.Body), nil
					}
					return res.Body, nil
				},
			})
		}
	}
	return out
}

// splitCSV splits a comma separated flag, dropping blanks.
func splitCSV(v string) []string {
	var out []string
	for _, p := range strings.Split(v, ",") {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}

// apiAddValuedFlags are api add's flags that take a value, for
// hoistFlags: the name comes first on the command line, so flags and
// their values move ahead of it as pairs.
var apiAddValuedFlags = map[string]bool{
	"spec": true, "base-url": true, "include": true, "exclude": true,
	"methods": true, "description": true, "header": true,
}

// parseAPIHeader splits one --header KEY=VALUE flag.
func parseAPIHeader(h string) (string, string, error) {
	k, v, ok := strings.Cut(h, "=")
	if !ok || strings.TrimSpace(k) == "" {
		return "", "", fmt.Errorf("a header needs KEY=VALUE, got %q", h)
	}
	return k, v, nil
}

// apiAdd declares a new API through the daemon, the way adapter add
// does: the write lands in the declaration file the running daemon
// reads and the reload makes it callable at once.
func (a *App) apiAdd(ctx context.Context, args []string) error {
	args = hoistFlags(args, apiAddValuedFlags)
	fs := newFlagSet("api")
	spec := fs.String("spec", "", "the OpenAPI document: a path or a URL")
	baseURL := fs.String("base-url", "", "override the server URL the document declares")
	include := fs.String("include", "", "only operations matching these words, comma separated")
	exclude := fs.String("exclude", "", "drop operations matching these words")
	methods := fs.String("methods", "", "HTTP methods to expose; 'all' for every one (default: read-only)")
	deprecated := fs.Bool("deprecated", false, "include operations the document marks deprecated")
	desc := fs.String("description", "", "one line shown beside the namespace")
	header := newRepeated()
	fs.Var(header, "header", "a header sent with every operation call, as KEY=VALUE (repeatable)")
	replace := fs.Bool("replace", false, "overwrite a declaration of the same name")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}
	if fs.NArg() == 0 {
		return fmt.Errorf("usage: mcpx api add <name> --spec <path-or-url> [--base-url <url>] [--include a,b] [--exclude c] [--methods all] [--deprecated] [--header K=V]... [--description ...] [--replace]")
	}
	if strings.TrimSpace(*spec) == "" {
		return fmt.Errorf("an API needs --spec <path-or-url>")
	}
	name := fs.Arg(0)
	// A local spec path is resolved against THIS process's working
	// directory before it is sent to the daemon: the daemon (and the
	// API server it spawns) may run from a different directory, so a
	// caller-relative path persisted unchanged would load the wrong
	// document or none at all (Codex P2, PR #4637). URLs pass through.
	specVal := *spec
	if !strings.HasPrefix(specVal, "http://") && !strings.HasPrefix(specVal, "https://") {
		if abs, aerr := filepath.Abs(specVal); aerr == nil {
			specVal = abs
		}
	}
	body := map[string]any{
		"name": name, "spec": specVal, "replace": *replace,
	}
	if *baseURL != "" {
		body["baseUrl"] = *baseURL
	}
	if v := splitCSV(*include); len(v) > 0 {
		body["include"] = v
	}
	if v := splitCSV(*exclude); len(v) > 0 {
		body["exclude"] = v
	}
	if v := splitCSV(*methods); len(v) > 0 {
		body["methods"] = v
	}
	if *deprecated {
		body["includeDeprecated"] = true
	}
	if *desc != "" {
		body["description"] = *desc
	}
	if len(header.values) > 0 {
		headers := map[string]string{}
		for _, h := range header.values {
			k, v, err := parseAPIHeader(h)
			if err != nil {
				return err
			}
			headers[k] = v
		}
		body["headers"] = headers
	}
	c, err := a.ensure(ctx)
	if err != nil {
		return err
	}
	raw, err := c.Do(ctx, http.MethodPost, "/v1/apis", mustMarshal(body))
	if err != nil {
		return err
	}
	var out map[string]any
	if err := json.Unmarshal(raw, &out); err != nil {
		return err
	}
	if a.JSON {
		return a.out(out)
	}
	fmt.Printf("added api %s to %v; it is live now\n", name, out["written"])
	return nil
}

// apiRemove removes a declaration through the daemon, mirroring adapter
// remove: the file that defines it is the one edited.
func (a *App) apiRemove(ctx context.Context, args []string) error {
	fs := newFlagSet("api")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}
	if fs.NArg() == 0 {
		return fmt.Errorf("usage: mcpx api remove <name>")
	}
	name := fs.Arg(0)
	c, err := a.ensure(ctx)
	if err != nil {
		return err
	}
	raw, err := c.Do(ctx, http.MethodDelete, "/v1/apis/"+urlEscape(name), nil)
	if err != nil {
		return err
	}
	var out map[string]any
	if err := json.Unmarshal(raw, &out); err != nil {
		return err
	}
	if a.JSON {
		return a.out(out)
	}
	fmt.Printf("removed api %s from %v; it is gone from the running daemon\n", name, out["written"])
	return nil
}
