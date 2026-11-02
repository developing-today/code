package cli

import (
	"context"
	"fmt"
	"sort"
	"strings"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/logstore"
	"github.com/dezren39/mcpx/internal/registry"
)

// The slow-remote doctor check (#1936): mcpx already measures every call a
// remote server takes — the log's per-tool latency and error figures — and
// already knows, through the registry, whether that same server could be
// hosted locally. This check joins the two: a configured remote (url)
// server that is measurably slow at real volume gets a warning carrying
// its own numbers, and a concrete local alternative when one exists.
//
// The gates are stated rather than tuned in secret, because a doctor
// warning a person cannot reproduce is a warning they learn to ignore.
const (
	// slowRemoteFloorCalls is the volume floor: below it the latency
	// figures are a handful of calls and prove nothing.
	slowRemoteFloorCalls = 50
	// slowRemoteP95Ms is the latency bar: a slowest-tool p95 at or above
	// it, at volume, is a round trip worth removing.
	slowRemoteP95Ms = 500
	// slowRemoteErrRate is the error bar: at or above it, at volume, the
	// remote is failing often enough that the transport is a suspect.
	slowRemoteErrRate = 0.05
)

// remoteStat is one server's measured figures, aggregated over its tools.
type remoteStat struct {
	Calls   int
	Errors  int
	ErrRate float64
	// P95Ms is the highest per-tool p95 the server recorded — a real
	// measured percentile of a real tool, not an average of percentiles.
	P95Ms float64
}

// aggregateRemoteStats folds per-tool call stats into per-server figures.
func aggregateRemoteStats(rows []logstore.CallStat) map[string]remoteStat {
	out := map[string]remoteStat{}
	for _, r := range rows {
		st := out[r.Server]
		st.Calls += r.Calls
		st.Errors += r.Errors
		if r.P95 > st.P95Ms {
			st.P95Ms = r.P95
		}
		out[r.Server] = st
	}
	for name, st := range out {
		if st.Calls > 0 {
			st.ErrRate = float64(st.Errors) / float64(st.Calls)
			out[name] = st
		}
	}
	return out
}

// slowRemoteChecks is the check itself, over inputs: the configured
// servers, the measured stats by server name, and the local package the
// registry offers per server name ("" when it offers none, or when the
// registry could not be asked — the figures are reported either way).
//
// Read-only by construction: it builds check rows and nothing else. A
// local (command) server is never flagged — it has no round trip to
// remove — and a remote below the volume floor is silent for the same
// reason a check that disappears differs from one that passed: there is
// simply no row, rather than an ok row claiming health unmeasured.
func slowRemoteChecks(servers []*config.Resolved, stats map[string]remoteStat, localPkg map[string]string) []check {
	var out []check
	for _, s := range servers {
		if s == nil || s.Server == nil || s.URL == "" {
			continue // not a remote server
		}
		st, ok := stats[s.Name]
		if !ok || st.Calls < slowRemoteFloorCalls {
			continue // below the volume floor: unmeasured, not healthy
		}
		if st.P95Ms < slowRemoteP95Ms && st.ErrRate < slowRemoteErrRate {
			continue // at volume and fine
		}
		detail := fmt.Sprintf("%s (%s): %d calls, %d errors (%.1f%%), slowest-tool p95 %.0fms",
			s.Name, s.URL, st.Calls, st.Errors, st.ErrRate*100, st.P95Ms)
		fix := "the numbers above are what the remote round trip costs; " +
			"if that latency or failure rate hurts, a local alternative is worth testing"
		if pkg := localPkg[s.Name]; pkg != "" {
			fix = fmt.Sprintf("host it locally instead: `mcpx registry add %s --write` installs %s; "+
				"the numbers above are what the round trip costs today", s.Name, pkg)
		}
		out = append(out, check{"remote latency", "warn", detail, fix})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Detail < out[j].Detail })
	return out
}

// doctorSlowRemotes runs the check for CmdDoctor: resolve the configured
// servers, aggregate the log's call stats, and — only for a server the
// check will actually flag — ask the registry whether it offers a local
// package. A registry that cannot be reached costs the recommendation,
// never the report.
func (a *App) doctorSlowRemotes(ctx context.Context) []check {
	cfg, err := config.Load(a.ConfigPath)
	if err != nil || cfg == nil {
		return nil
	}
	servers, err := cfg.ResolveAll()
	if err != nil {
		return nil
	}
	var remotes []*config.Resolved
	for _, s := range servers {
		if s.URL != "" {
			remotes = append(remotes, s)
		}
	}
	if len(remotes) == 0 {
		return nil
	}
	st, err := a.openStore("")
	if err != nil {
		return nil
	}
	defer st.Close()
	rows, err := st.Calls(logstore.Query{})
	if err != nil {
		return nil
	}
	stats := aggregateRemoteStats(rows)

	// Ask the registry only about servers the figures already flag:
	// a search per configured remote on every doctor run would make
	// doctor slow in exactly the way this check complains about.
	localPkg := map[string]string{}
	for _, c := range slowRemoteChecks(remotes, stats, localPkg) {
		name := strings.SplitN(c.Detail, " ", 2)[0]
		if pkg := a.registryLocalPackage(ctx, name); pkg != "" {
			localPkg[name] = pkg
		}
	}
	return slowRemoteChecks(remotes, stats, localPkg)
}

// registryLocalPackage returns a printable local package ("npm package
// <id>") when the registry has an entry under this exact name carrying at
// least one installable package, and "" otherwise — including on any
// error, because the caller treats "" as "report only".
func (a *App) registryLocalPackage(ctx context.Context, name string) string {
	set := a.Settings()
	client := registry.New(set.String("registry.url"), registry.Options{
		Timeout:  set.Duration("registry.timeout"),
		PageSize: set.Int("registry.pageSize"),
		MaxPages: set.Int("registry.maxPages"),
	})
	res, err := client.Search(ctx, name, set.Int("registry.limit"))
	if err != nil {
		return ""
	}
	for _, srv := range res.Servers {
		if !strings.EqualFold(srv.Name, name) {
			continue
		}
		for _, p := range srv.Packages {
			if p.Identifier != "" {
				return p.RegistryType + " package " + p.Identifier
			}
		}
	}
	return ""
}
