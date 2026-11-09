package api_test

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
)

// The dry-run report (#1972 step 2): a request a future policy would
// refuse — an anonymous caller asking for an admin-tier operation —
// still succeeds in the permissive stage, and it shows up in the report
// under its operation, its would-require tier, and its principal.
func TestAuthzReportCountsAWouldRefuseRequest(t *testing.T) {
	api.ClearDecisions()
	admin, ok := api.ByName("restart")
	if !ok {
		t.Fatal("restart operation missing from the table")
	}
	if admin.EffectivePermission() != api.PermissionAdmin {
		t.Fatalf("restart tier = %q, want admin (the would-refuse fixture)", admin.EffectivePermission())
	}
	if !api.Authorize(api.AnonymousPrincipal(), admin) {
		t.Fatal("Authorize refused an admin request in the permissive stage; step 2 must not refuse")
	}
	health, ok := api.ByName("health")
	if !ok {
		t.Fatal("health operation missing from the table")
	}
	api.Authorize(api.UnixPeerPrincipal(1000), health)
	api.Authorize(api.UnixPeerPrincipal(1000), health)

	report := api.ReportDecisions()
	if report.Stage != "permissive" {
		t.Errorf("report stage = %q, want permissive", report.Stage)
	}
	if report.Total != 3 || report.Allowed != 3 {
		t.Errorf("report total/allowed = %d/%d, want 3/3", report.Total, report.Allowed)
	}
	if report.Since == nil || report.Until == nil {
		t.Error("report window is unbounded: since/until missing for a non-empty record")
	}
	var sawAdmin, sawHealth bool
	for _, row := range report.ByOperation {
		switch row.Op {
		case "restart":
			sawAdmin = true
			if row.Permission != api.PermissionAdmin || row.Count != 1 || row.Allowed != 1 {
				t.Errorf("restart row = %+v, want admin tier, count 1, allowed 1", row)
			}
		case "health":
			sawHealth = true
			if row.Count != 2 {
				t.Errorf("health row count = %d, want 2", row.Count)
			}
		}
	}
	if !sawAdmin || !sawHealth {
		t.Errorf("report by_operation missing rows: admin=%v health=%v (%+v)", sawAdmin, sawHealth, report.ByOperation)
	}
	var sawAnon, sawUnix bool
	for _, row := range report.ByPrincipal {
		switch {
		case row.Surface == "anonymous":
			sawAnon = true
			if row.Count != 1 {
				t.Errorf("anonymous row count = %d, want 1", row.Count)
			}
		case row.Surface == "unix" && row.ID == "uid=1000":
			sawUnix = true
			if row.Count != 2 {
				t.Errorf("unix uid=1000 row count = %d, want 2", row.Count)
			}
		}
	}
	if !sawAnon || !sawUnix {
		t.Errorf("report by_principal missing rows: anonymous=%v unix=%v (%+v)", sawAnon, sawUnix, report.ByPrincipal)
	}
}

// The record carries no credential material: a Decision has no field a
// secret could be written into, and the marshalled record and report
// contain neither a sentinel secret value nor any credential-shaped
// field name.
func TestAuthzReportRecordsNoSecret(t *testing.T) {
	api.ClearDecisions()
	op, ok := api.ByName("health")
	if !ok {
		t.Fatal("health operation missing from the table")
	}
	api.Authorize(api.UnixPeerPrincipal(1000), op)

	const sentinel = "s3cr3t-token-value"
	for name, blob := range map[string]any{
		"decisions": api.Decisions(),
		"report":    api.ReportDecisions(),
	} {
		raw, err := json.Marshal(blob)
		if err != nil {
			t.Fatalf("marshal %s: %v", name, err)
		}
		if strings.Contains(string(raw), sentinel) {
			t.Errorf("%s contains the sentinel secret value: %s", name, raw)
		}
		for _, field := range []string{`"token"`, `"secret"`, `"password"`, `"credential"`, `"authorization"`} {
			if strings.Contains(string(raw), field) {
				t.Errorf("%s carries a credential-shaped field %s: %s", name, field, raw)
			}
		}
	}
}

// The clear is the operator's reset for the dry-run window: after it,
// the record and the report are empty, and recording resumes.
func TestAuthzClearDecisionsEmptiesTheReport(t *testing.T) {
	api.ClearDecisions()
	op, ok := api.ByName("health")
	if !ok {
		t.Fatal("health operation missing from the table")
	}
	api.Authorize(api.UnixPeerPrincipal(1000), op)
	if got := api.ReportDecisions().Total; got != 1 {
		t.Fatalf("report total before clear = %d, want 1", got)
	}
	api.ClearDecisions()
	if got := len(api.Decisions()); got != 0 {
		t.Errorf("decisions after clear = %d, want 0", got)
	}
	report := api.ReportDecisions()
	if report.Total != 0 || report.Since != nil || report.Until != nil {
		t.Errorf("report after clear = %+v, want an empty, unbounded report", report)
	}
	if len(report.ByOperation) != 0 || len(report.ByPrincipal) != 0 {
		t.Errorf("report rows after clear = %+v / %+v, want none", report.ByOperation, report.ByPrincipal)
	}
}
