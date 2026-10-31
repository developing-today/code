package cli

import (
	"encoding/base64"
	"os"
	"path/filepath"
	"testing"

	"github.com/dezren39/mcpx/internal/artifacts"
	"github.com/dezren39/mcpx/internal/execsvc"
)

// TestApplyFrameSameNamedArtifactsNeverOverwrite is the #1605 regression
// test for the streaming delivery path: two artifacts that sanitise to the
// same file name must both survive, suffixed the way Store.Export suffixes,
// with each artifact's Path pointing at its own body.
func TestApplyFrameSameNamedArtifactsNeverOverwrite(t *testing.T) {
	dir := t.TempDir()
	a := &App{}
	res := &execsvc.Result{}
	eo := execOptions{ArtifactsDir: dir}
	pending := map[string]*os.File{}

	deliver := func(id, name, body string) {
		t.Helper()
		art := &execsvc.Artifact{Meta: artifacts.Meta{ID: id, Name: name}}
		if err := a.applyFrame(res, execsvc.Frame{Type: execsvc.FrameArtifact, Artifact: art}, eo, pending); err != nil {
			t.Fatal(err)
		}
		data := base64.StdEncoding.EncodeToString([]byte(body))
		if err := a.applyFrame(res, execsvc.Frame{Type: execsvc.FrameArtifactData, ID: id, Data: data}, eo, pending); err != nil {
			t.Fatal(err)
		}
		if err := a.applyFrame(res, execsvc.Frame{Type: execsvc.FrameArtifactEnd, ID: id}, eo, pending); err != nil {
			t.Fatal(err)
		}
	}
	// "my shot.png" and "my-shot.png" both sanitise to my-shot.png.
	deliver("id1", "my shot.png", "FIRST-BODY")
	deliver("id2", "my-shot.png", "SECOND-BODY")

	if len(res.Artifacts) != 2 {
		t.Fatalf("artifacts = %d, want 2", len(res.Artifacts))
	}
	first, second := res.Artifacts[0].Path, res.Artifacts[1].Path
	if first == second {
		t.Fatalf("both artifacts point at %q; the first body was overwritten", first)
	}
	if filepath.Base(first) != "my-shot.png" || filepath.Base(second) != "my-shot-1.png" {
		t.Fatalf("paths = %q, %q; want my-shot.png and my-shot-1.png", first, second)
	}
	for path, want := range map[string]string{first: "FIRST-BODY", second: "SECOND-BODY"} {
		got, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		if string(got) != want {
			t.Fatalf("%s holds %q, want %q", path, got, want)
		}
	}
}

// TestFetchArtifactsSameNamedNeverOverwrite covers the second #1605
// delivery path: inline bodies pulled by fetchArtifacts follow the same
// never-overwrite rule.
func TestFetchArtifactsSameNamedNeverOverwrite(t *testing.T) {
	dir := t.TempDir()
	a := &App{client: &Client{}, connected: true}
	enc := func(s string) string { return base64.StdEncoding.EncodeToString([]byte(s)) }
	res := &execsvc.Result{Artifacts: []execsvc.Artifact{
		{Meta: artifacts.Meta{ID: "id1", Name: "shot.png"}, Data: enc("FIRST-BODY")},
		{Meta: artifacts.Meta{ID: "id2", Name: "shot.png"}, Data: enc("SECOND-BODY")},
	}}
	if err := a.fetchArtifacts(res, execOptions{ArtifactsDir: dir}); err != nil {
		t.Fatal(err)
	}
	first, second := res.Artifacts[0].Path, res.Artifacts[1].Path
	if first == second {
		t.Fatalf("both artifacts point at %q; the first body was overwritten", first)
	}
	if filepath.Base(first) != "shot.png" || filepath.Base(second) != "shot-1.png" {
		t.Fatalf("paths = %q, %q; want shot.png and shot-1.png", first, second)
	}
	for path, want := range map[string]string{first: "FIRST-BODY", second: "SECOND-BODY"} {
		got, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		if string(got) != want {
			t.Fatalf("%s holds %q, want %q", path, got, want)
		}
	}
}
