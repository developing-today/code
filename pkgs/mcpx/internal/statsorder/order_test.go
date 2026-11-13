package statsorder

import "testing"

func TestLess(t *testing.T) {
	tests := []struct {
		name                           string
		aTotal, bTotal                 float64
		aServer, aTool, bServer, bTool string
		want                           bool
	}{
		{"time descending", 2, 1, "z", "z", "a", "a", true},
		{"time reverse", 1, 2, "a", "a", "z", "z", false},
		{"server ascending", 1, 1, "a", "z", "b", "a", true},
		{"server reverse", 1, 1, "b", "a", "a", "z", false},
		{"tool ascending", 1, 1, "a", "a", "a", "b", true},
		{"tool reverse", 1, 1, "a", "b", "a", "a", false},
		{"equal", 1, 1, "a", "b", "a", "b", false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := Less(tt.aTotal, tt.aServer, tt.aTool, tt.bTotal, tt.bServer, tt.bTool); got != tt.want {
				t.Errorf("Less() = %v, want %v", got, tt.want)
			}
		})
	}
}
