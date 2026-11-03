package eventstore

import (
	"database/sql"
	"encoding/json"
	"math"
	"sort"
)

// CallStat is one server+tool pair's call history — the same shape as
// logstore.CallStat, deliberately: this dimension is the proof that a
// hand-written statistic becomes a query (#1850). The calls dimension
// is re-expressed over events of kind "mcp.call" whose data carries
// {server, tool, ok} and whose value is the call's duration in ms.
type CallStat struct {
	Server  string  `json:"server"`
	Tool    string  `json:"tool"`
	Calls   int     `json:"calls"`
	Errors  int     `json:"errors"`
	ErrRate float64 `json:"errorRate"`
	P50     float64 `json:"p50Ms"`
	P95     float64 `json:"p95Ms"`
	P99     float64 `json:"p99Ms"`
	Max     float64 `json:"maxMs"`
	TotalMs float64 `json:"totalMs"`
}

// CallStats aggregates mcp.call events by server and tool. The
// aggregation mirrors logstore.Calls exactly — same grouping, same
// nearest-rank percentiles, same ordering — and the package test
// proves the two agree on shared fixture data.
func (s *Store) CallStats() ([]CallStat, error) {
	rows, err := s.db.Query(
		`SELECT e.value, e.data FROM event e JOIN kind k ON k.id = e.kind_id
		 WHERE k.name = 'mcp.call' ORDER BY e.id`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	type key struct{ server, tool string }
	durations := map[key][]float64{}
	counts := map[key]*CallStat{}
	for rows.Next() {
		var value sql.NullFloat64
		var data sql.NullString
		if err := rows.Scan(&value, &data); err != nil {
			return nil, err
		}
		var payload struct {
			Server string `json:"server"`
			Tool   string `json:"tool"`
			OK     *bool  `json:"ok"`
		}
		if data.Valid && data.String != "" {
			if err := json.Unmarshal([]byte(data.String), &payload); err != nil {
				return nil, err
			}
		}
		k := key{payload.Server, payload.Tool}
		st := counts[k]
		if st == nil {
			st = &CallStat{Server: k.server, Tool: k.tool}
			counts[k] = st
		}
		st.Calls++
		if payload.OK != nil && !*payload.OK {
			st.Errors++
		}
		if value.Valid {
			durations[k] = append(durations[k], value.Float64)
			st.TotalMs += value.Float64
		}
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}

	out := make([]CallStat, 0, len(counts))
	for k, st := range counts {
		d := durations[k]
		sort.Float64s(d)
		st.P50, st.P95, st.P99 = percentile(d, 50), percentile(d, 95), percentile(d, 99)
		if len(d) > 0 {
			st.Max = d[len(d)-1]
		}
		if st.Calls > 0 {
			st.ErrRate = float64(st.Errors) / float64(st.Calls)
		}
		out = append(out, *st)
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].TotalMs != out[j].TotalMs {
			return out[i].TotalMs > out[j].TotalMs
		}
		return out[i].Server+out[i].Tool < out[j].Server+out[j].Tool
	})
	return out, nil
}

// percentile is logstore's nearest-rank percentile, kept identical so
// the two implementations report the same numbers on the same data.
func percentile(sorted []float64, p float64) float64 {
	if len(sorted) == 0 {
		return 0
	}
	rank := int(math.Ceil(p/100*float64(len(sorted)))) - 1
	if rank < 0 {
		rank = 0
	}
	if rank >= len(sorted) {
		rank = len(sorted) - 1
	}
	return sorted[rank]
}
