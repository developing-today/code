package elicit

// RefusalCode is the machine-readable reason a call on the mcpx call
// path was refused (#1927, item 6). A refused call's human-readable
// error says what happened in prose; the code is what a caller branches
// on, so the codes are defined exactly once, here, and every refusal
// site and surface (daemon error body, exec, diagnose) reuses them
// instead of inventing its own spelling.
//
// The empty RefusalCode is meaningful: it is what an allowed call
// carries, and what a failure that is not a refusal carries. Only a
// refusal has a code.
type RefusalCode string

const (
	// RefusalUnanswered: a confirmation question was asked and nobody
	// answered before the deadline. For a destructive confirmation
	// that refuses the call: doing nothing is the destructive thing,
	// so silence is a refusal, never an approval.
	RefusalUnanswered RefusalCode = "confirmation_unanswered"
	// RefusalDeclined: the answerer explicitly declined the
	// elicitation.
	RefusalDeclined RefusalCode = "elicitation_declined"
	// RefusalCancelled: the answerer cancelled the elicitation
	// ("not now") rather than declining it outright.
	RefusalCancelled RefusalCode = "elicitation_cancelled"
	// RefusalDenied: the elicitation was answered, but the answer did
	// not grant what was asked (a confirmation answered with
	// confirm=false, or an accept whose content is unusable).
	RefusalDenied RefusalCode = "confirmation_denied"
	// RefusalUnavailable: the policy asked for a confirmation but no
	// question broker exists to answer it, so the call is refused
	// rather than waited out or waved through.
	RefusalUnavailable RefusalCode = "confirmation_unavailable"
	// RefusalAnnotationsUnreadable: the gate could not read the
	// tool's annotations, and deciding from an empty or stale list
	// would fail open, so the call is refused.
	RefusalAnnotationsUnreadable RefusalCode = "annotations_unreadable"
)

// RefusalCodeForAnswer maps a non-confirming elicitation answer to its
// refusal code. Callers use it only once they know the answer did not
// confirm the call; an answer is never re-examined here for whether it
// confirmed, only for how it refused.
func RefusalCodeForAnswer(ans Answer) RefusalCode {
	switch ans.Action {
	case Decline:
		return RefusalDeclined
	case Cancel:
		return RefusalCancelled
	default:
		// Accept with content that did not grant the confirmation, or
		// an action outside the known three: the question was answered
		// and the answer was not consent.
		return RefusalDenied
	}
}
