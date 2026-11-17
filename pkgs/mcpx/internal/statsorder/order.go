// Package statsorder defines the common ordering of call statistics.
package statsorder

// Less orders total time descending, then server and tool ascending.
func Less(aTotal float64, aServer, aTool string, bTotal float64, bServer, bTool string) bool {
	if aTotal != bTotal {
		return aTotal > bTotal
	}
	if aServer != bServer {
		return aServer < bServer
	}
	return aTool < bTool
}
