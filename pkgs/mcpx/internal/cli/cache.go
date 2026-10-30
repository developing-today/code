package cli

import (
	"context"
	"fmt"
	"os"
	"sort"
	"text/tabwriter"

	"github.com/dezren39/mcpx/internal/cache"
	"github.com/dezren39/mcpx/internal/defaults"
)

// CmdCache is the listing and pruning surface for the one cache
// mechanism (#1908): `mcpx cache` lists every kind and entry under the
// cache root, `mcpx cache prune` runs eviction with the configured
// bounds, and `mcpx cache remove <kind> <key>` deletes one entry
// (refusing while a running process leases it).
//
// The command is deliberately local: it reads the filesystem the same
// way the writers do, and never starts or needs the daemon — the same
// decision `status` made (#1866).
func (a *App) CmdCache(ctx context.Context, args []string) error {
	fs := newFlagSet("cache")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}
	rest := fs.Args()
	c := cache.Open(a.Paths.Cache)
	lim := cache.Limits{
		MaxBytes:      defaults.CacheMaxBytes,
		MaxAge:        defaults.CacheMaxAge,
		LeaseStaleAge: defaults.CacheLeaseStaleAge,
	}
	switch {
	case len(rest) == 0:
		return a.cacheList(c, lim)
	case rest[0] == "prune":
		kinds, err := cacheKinds(c)
		if err != nil {
			return err
		}
		for _, kind := range kinds {
			removed := c.Evict(kind, lim)
			fmt.Printf("%s: pruned %d entr(ies)\n", kind, len(removed))
		}
		return nil
	case rest[0] == "remove" && len(rest) == 3:
		if err := c.Remove(rest[1], rest[2], lim.LeaseStaleAge); err != nil {
			return err
		}
		fmt.Printf("removed %s/%s\n", rest[1], rest[2])
		return nil
	default:
		return fmt.Errorf("usage: mcpx cache [prune | remove <kind> <key>]")
	}
}

func cacheKinds(c *cache.Cache) ([]string, error) {
	ents, err := os.ReadDir(c.Root)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var kinds []string
	for _, e := range ents {
		if e.IsDir() {
			kinds = append(kinds, e.Name())
		}
	}
	sort.Strings(kinds)
	return kinds, nil
}

func (a *App) cacheList(c *cache.Cache, lim cache.Limits) error {
	kinds, err := cacheKinds(c)
	if err != nil {
		return err
	}
	type row struct {
		Kind   string `json:"kind"`
		Key    string `json:"key"`
		Size   int64  `json:"size"`
		Leased bool   `json:"leased"`
		Path   string `json:"path"`
	}
	var rows []row
	for _, kind := range kinds {
		entries, err := c.List(kind, lim.LeaseStaleAge)
		if err != nil {
			return err
		}
		for _, e := range entries {
			rows = append(rows, row{e.Kind, e.Key, e.Size, e.Leased, e.Path})
		}
	}
	if a.JSON {
		return a.out(rows)
	}
	if len(rows) == 0 {
		fmt.Printf("cache: empty (%s)\n", c.Root)
		return nil
	}
	tw := tabwriter.NewWriter(os.Stdout, 0, 0, 2, ' ', 0)
	fmt.Fprintln(tw, "KIND\tKEY\tBYTES\tLEASED\tPATH")
	for _, r := range rows {
		leased := ""
		if r.Leased {
			leased = "yes"
		}
		fmt.Fprintf(tw, "%s\t%s\t%d\t%s\t%s\n", r.Kind, r.Key, r.Size, leased, r.Path)
	}
	return tw.Flush()
}
