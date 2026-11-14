package cli

import (
	"context"
	"fmt"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/wantlog"
)

// CmdWant records an explicit capability request and lists the grouped
// local wants: explicit requests plus automatically recorded empty
// registry searches.
//
// The record is local-only -- one append-only JSONL file under the state
// directory -- and is never transmitted anywhere (#1936).
// `mcpx want calendar mcp` records; `mcpx want list` lists;
// `mcpx want request <text>` is the explicit spelling.
func (a *App) CmdWant(ctx context.Context, args []string) error {
	// Like registry search, flags may trail the query: Go's parser stops
	// at the first non-flag, so `want list --kind request` would
	// otherwise read "--kind" as part of the request text.
	args = hoistFlags(args, map[string]bool{"kind": true, "limit": true})
	fs := newFlagSet("want")
	kind := fs.String("kind", "", "list only this kind: request or search-miss")
	limit := fs.Int("limit", 0, "list at most this many groups (default all)")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}

	store := wantlog.New(wantlog.PathFor(a.wantStateDir()))
	rest := fs.Args()
	sub := ""
	if len(rest) > 0 {
		sub, rest = rest[0], rest[1:]
	}
	switch sub {
	case "request", "add":
		return a.wantRequest(store, strings.Join(rest, " "))
	case "list", "ls":
		return a.wantList(store, *kind, *limit)
	case "":
		return a.wantList(store, *kind, *limit)
	default:
		// The first word of a request is not a verb: `mcpx want calendar
		// mcp` records "calendar mcp" rather than failing on "calendar".
		return a.wantRequest(store, strings.Join(append([]string{sub}, rest...), " "))
	}
}

func (a *App) wantRequest(store *wantlog.Store, text string) error {
	e, err := store.Append(wantlog.KindRequest, text)
	if err != nil {
		return err
	}
	if a.JSON {
		return a.out(e)
	}
	fmt.Printf("Recorded capability request %q.\n", e.Query)
	return nil
}

func (a *App) wantList(store *wantlog.Store, kind string, limit int) error {
	switch kind {
	case "", wantlog.KindRequest, wantlog.KindSearchMiss:
	default:
		return fmt.Errorf("unknown want kind %q; want request or search-miss", kind)
	}
	groups, err := store.Summaries(kind, limit)
	if err != nil {
		return err
	}
	if a.JSON {
		return a.out(groups)
	}
	if len(groups) == 0 {
		fmt.Println("No capability requests or empty searches recorded yet.")
		return nil
	}
	for _, g := range groups {
		fmt.Printf("%-11s %3d  first %s  last %s\n  %s\n",
			g.Kind, g.Count,
			g.FirstSeen.Format("2006-01-02 15:04"),
			g.LastSeen.Format("2006-01-02 15:04"),
			g.Query)
	}
	return nil
}

// wantStateDir is the state directory the want log lives under: the
// resolved paths when the invocation has them, the environment-resolved
// default otherwise (a zero-value App in tests, for example).
func (a *App) wantStateDir() string {
	if a != nil && a.Paths.State != "" {
		return a.Paths.State
	}
	return wantlog.ResolveStateDir()
}
