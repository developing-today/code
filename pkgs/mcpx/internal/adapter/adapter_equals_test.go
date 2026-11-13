package adapter

import (
	"reflect"
	"testing"
)

// The oracle is argv itself: an equals-bound parameter must reach the
// program attached (--flag=value), because programs with optional
// values read the separated form as a different operation entirely.

func TestBuildArgsEqualsRendering(t *testing.T) {
	tool := ToolSpec{
		Name: "run",
		Params: []Param{
			{Name: "exec_path", Flag: "--exec-path", Equals: true},
			{Name: "config", Flag: "--config"},
		},
	}
	got, _, err := tool.buildArgs(map[string]any{
		"exec_path": "/tmp/bin",
		"config":    "a.yaml",
	})
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"--exec-path=/tmp/bin", "--config", "a.yaml"}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("argv: got %q, want %q", got, want)
	}
}

func TestBuildArgsEqualsRepeat(t *testing.T) {
	tool := ToolSpec{
		Name:   "run",
		Params: []Param{{Name: "define", Flag: "--define", Equals: true, Repeat: true, Type: "array"}},
	}
	got, _, err := tool.buildArgs(map[string]any{"define": []any{"a=1", "b=2"}})
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"--define=a=1", "--define=b=2"}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("argv: got %q, want %q", got, want)
	}
}
