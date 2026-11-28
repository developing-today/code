package api

// Permission is the authorisation tier an operation declares, from #1968:
// a principal per surface and a permission per operation, declared
// permissively. The vocabulary is tiers, not per-operation scopes — three
// names a person can hold in their head, tagged onto the operations table
// in one place (the Op declaration), so a future policy has something to
// constrain and a future surface something to populate.
//
// The tiers:
//
//   - PermissionRead: a plain read; it changes nothing.
//   - PermissionRun: anything that is not a plain read — it mutates state
//     or runs something, but does not reconfigure the daemon itself.
//   - PermissionAdmin: it changes the daemon rather than reading or using
//     it (the operations that already carry Admin).
//
// Declared permissively means the check built on this vocabulary always
// answers yes today (see Principal.Allows). The declaration is the
// deliverable; enforcement is a later issue and deliberately not here.
type Permission string

const (
	PermissionRead  Permission = "read"
	PermissionRun   Permission = "run"
	PermissionAdmin Permission = "admin"
)

// DerivePermission computes the tier an operation's existing flags imply:
// Admin is the daemon-changing tier, Mutating or Destructive is the run
// tier, and a plain read is the read tier. It exists so a test can pin
// the declared Permission of every operation against the flags it already
// carries — a declaration that drifts from the flags is a deliberate diff
// in the test, never an accident.
func DerivePermission(o Op) Permission {
	switch {
	case o.Admin:
		return PermissionAdmin
	case o.Mutating || o.Destructive:
		return PermissionRun
	default:
		return PermissionRead
	}
}

// EffectivePermission is the permission a check consults: the declared
// field when the operation declares one, the derived tier otherwise. The
// registry test requires every operation to declare one, so the fallback
// is a safety net for operations constructed outside the table (tests,
// hand-built calls), not a path the daemon's own operations take.
func (o Op) EffectivePermission() Permission {
	if o.Permission != "" {
		return o.Permission
	}
	return DerivePermission(o)
}

// ByRoute finds the operation a mux pattern names ("GET /v1/elicit/{id}").
// The daemon's authorisation funnel uses it to attach the matched
// operation's permission to the decision it records for a request.
func ByRoute(route string) (Op, bool) {
	for _, o := range Ops() {
		if o.Route() == route {
			return o, true
		}
	}
	return Op{}, false
}
