package daemon

import (
	"fmt"
	"net/http"
	"net/url"
	"strconv"
)

// parseOptionalIntQuery parses an optional integer query parameter.
//
// Absent means the caller applies its configured default. Present and valid
// returns the parsed value (including zero and negatives — callers keep their
// existing defaulting for those). Present but unparseable (including empty,
// "banana", "2.7", "1e3") is a 400 naming the parameter and the bad value,
// matching the #1679/#2223 house rule that a silently dropped bound reads as
// a real answer.
func parseOptionalIntQuery(w http.ResponseWriter, q url.Values, name string) (int, bool) {
	if _, present := q[name]; !present {
		return 0, true
	}
	raw := q.Get(name)
	n, err := strconv.Atoi(raw)
	if err != nil {
		writeErr(w, http.StatusBadRequest, fmt.Errorf("invalid %s %q: must be an integer", name, raw))
		return 0, false
	}
	return n, true
}
