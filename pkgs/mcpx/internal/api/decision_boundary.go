package api

// Decision audit versus retained feedback: access, retention and field
// selection boundary (#13577).
//
// Decision events and optional feedback capture are separate stores with
// separate readers. This file is the spec; the *_test.go files pin it with
// synthetic markers so a future field cannot silently cross the boundary.
//
// Authorized readers:
//   - Decision audit export (GET /v1/authz/decisions, op authz_decisions):
//     PermissionRead. It is the operator's shared dry-run read; every
//     principal that may call it sees the same aggregate counts.
//   - Decision clear (DELETE /v1/authz/decisions, op authz_decisions_clear):
//     PermissionAdmin only.
//   - Feedback history (GET /v1/interactions, GET /v1/interactions/{id},
//     ops interactions_list / interactions_get): PermissionRead, user-owned
//     history. It returns the retained payloads below.
//   - Feedback submit (POST /v1/feedback, op feedback): PermissionRun.
//     It attaches scores/notes to an existing trace for adaptive routing.
//
// Retention:
//   - Decisions: in-memory only, bounded to the most recent
//     maxRecordedDecisions (see DecisionRetentionBound), oldest dropped
//     first, emptied on demand by ClearDecisions. Never persisted to disk,
//     never written into the feedback SQLite store.
//   - Feedback: persisted in the caller's SQLite feedback store
//     (logstore.FeedbackStore), user-owned. No automatic expiry or
//     migration rewrites it; rating behavior (ProvideFeedback) only fills
//     score/note columns on the trace the caller named.
//
// Field selection:
//   - A Decision carries exactly: principal surface + descriptive ID
//     (never a credential), operation name, permission tier, allowed answer,
//     and timestamp. It carries no input text, no output text, no tool
//     arguments or results, no user query, no capped response, no scores,
//     and no feedback notes.
//   - Feedback payloads (InteractionRecord: input, output, tools used,
//     scores, notes) are consumed only by per-tool ranking adjustment and
//     by the user-owned history reads above. They are never projected into
//     Decisions() or ReportDecisions(), and the shared audit export must
//     never contain them — including content from a denied (low-scored)
//     trace or any other sensitive content the caller retained.
//
// The forbidden-field lists live in decision_boundary_test.go, which iterates
// the Decision struct: a new field whose name reuses a feedback-payload or
// secret fragment fails the guard, present or future.

// DecisionRetentionBound reports the bounded window the decision record
// keeps: the most recent N decisions. It is the spec value the retention
// tests pin; change it here, not by editing the record trimming.
func DecisionRetentionBound() int {
	return maxRecordedDecisions
}
