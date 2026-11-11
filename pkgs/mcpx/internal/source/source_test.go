package source_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/source"
)

func opts(dir string) source.Options {
	return source.Options{Dir: dir, AllowDir: true, Probe: true}
}

func TestInlineTextIsUsedAsIs(t *testing.T) {
	got, err := source.Resolve(`console.log("hi")`, opts(t.TempDir()))
	if err != nil {
		t.Fatal(err)
	}
	if got.Kind != source.KindText || got.Text != `console.log("hi")` {
		t.Errorf("got %+v", got)
	}
}

func TestAValueNamingAnExistingFileIsReadAsOne(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "hook.ts")
	if err := os.WriteFile(path, []byte("export const x = 1;\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	got, err := source.Resolve("hook.ts", opts(dir))
	if err != nil {
		t.Fatal(err)
	}
	if got.Kind != source.KindFile {
		t.Fatalf("should have been read as a file: %+v", got)
	}
	if !strings.Contains(got.Text, "export const x") {
		t.Errorf("contents not read: %q", got.Text)
	}
}

func TestProbingCanBeTurnedOff(t *testing.T) {
	// Worth having for anyone whose snippets collide with filenames.
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "x"), []byte("FILE"), 0o644); err != nil {
		t.Fatal(err)
	}
	o := opts(dir)
	o.Probe = false
	got, _ := source.Resolve("x", o)
	if got.Kind != source.KindText || got.Text != "x" {
		t.Errorf("with probing off the value is literal: %+v", got)
	}
}

func TestExplicitPrefixesOverrideTheProbe(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "amb")
	if err := os.WriteFile(path, []byte("FROM FILE"), 0o644); err != nil {
		t.Fatal(err)
	}
	asText, err := source.Resolve("@text:amb", opts(dir))
	if err != nil {
		t.Fatal(err)
	}
	if asText.Text != "amb" || !asText.Explicit {
		t.Errorf("@text: should win over the probe: %+v", asText)
	}
	asFile, err := source.Resolve("@file:amb", opts(dir))
	if err != nil {
		t.Fatal(err)
	}
	if asFile.Text != "FROM FILE" {
		t.Errorf("@file: should read the file: %+v", asFile)
	}
}

func TestAMissingExplicitFileIsAnErrorRatherThanSilentText(t *testing.T) {
	_, err := source.Resolve("@file:/nope/missing.ts", opts(t.TempDir()))
	if err == nil {
		t.Fatal("naming a file that is not there should fail, not fall back to text")
	}
}

func TestADirectoryIsConcatenatedInNaturalOrder(t *testing.T) {
	// Byte order puts 10 before 9, which is wrong for precisely the case
	// directory concatenation exists to serve.
	dir := t.TempDir()
	for _, n := range []string{"2-b.ts", "10-c.ts", "1-a.ts"} {
		if err := os.WriteFile(filepath.Join(dir, n), []byte("// "+n), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	got, err := source.Resolve(dir, opts(dir))
	if err != nil {
		t.Fatal(err)
	}
	if got.Kind != source.KindDir {
		t.Fatalf("should be a directory read: %+v", got)
	}
	var order []string
	for _, f := range got.Files {
		order = append(order, filepath.Base(f))
	}
	want := "1-a.ts,2-b.ts,10-c.ts"
	if strings.Join(order, ",") != want {
		t.Errorf("natural order: got %v, want %s", order, want)
	}
}

func TestEachConcatenatedFileIsNamedInTheOutput(t *testing.T) {
	// A stack trace into a concatenation is otherwise unattributable.
	dir := t.TempDir()
	_ = os.WriteFile(filepath.Join(dir, "a.ts"), []byte("const a = 1;"), 0o644)
	_ = os.WriteFile(filepath.Join(dir, "b.ts"), []byte("const b = 2;"), 0o644)
	got, err := source.Resolve(dir, opts(dir))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Count(got.Text, "// --- ") != 2 {
		t.Errorf("both files should be attributed:\n%s", got.Text)
	}
}

func TestADirectoryIsRefusedWhereTheSettingDoesNotTakeOne(t *testing.T) {
	dir := t.TempDir()
	_ = os.WriteFile(filepath.Join(dir, "a.ts"), []byte("x"), 0o644)
	o := opts(dir)
	o.AllowDir = false
	_, err := source.Resolve(dir, o)
	if err == nil {
		t.Fatal("a directory should be refused when the setting does not take one")
	}
	if !strings.Contains(err.Error(), "sourceDirAllowed") {
		t.Errorf("the error should name the switch that changes it: %v", err)
	}
}

func TestShallowByDefaultAndRecursiveWhenAsked(t *testing.T) {
	dir := t.TempDir()
	_ = os.WriteFile(filepath.Join(dir, "top.ts"), []byte("t"), 0o644)
	sub := filepath.Join(dir, "sub")
	_ = os.MkdirAll(sub, 0o755)
	_ = os.WriteFile(filepath.Join(sub, "deep.ts"), []byte("d"), 0o644)

	shallow, err := source.Resolve(dir, opts(dir))
	if err != nil {
		t.Fatal(err)
	}
	if len(shallow.Files) != 1 {
		t.Errorf("shallow should see only the top file: %v", shallow.Files)
	}
	o := opts(dir)
	o.Recursive = true
	deep, err := source.Resolve(dir, o)
	if err != nil {
		t.Fatal(err)
	}
	if len(deep.Files) != 2 {
		t.Errorf("recursive should see both: %v", deep.Files)
	}
}

func TestDirectoryReadsSymlinkToRegularFileUsingLinkPath(t *testing.T) {
	dir := t.TempDir()
	target := filepath.Join(dir, "target.data")
	link := filepath.Join(dir, "alias.ts")
	if err := os.WriteFile(target, []byte("from target"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(target, link); err != nil {
		t.Skipf("symlinks are unavailable: %v", err)
	}

	o := opts(dir)
	o.Extensions = []string{".ts"}
	got, err := source.Resolve("@dir:"+dir, o)
	if err != nil {
		t.Fatal(err)
	}
	if len(got.Files) != 1 || got.Files[0] != link {
		t.Fatalf("files should contain the matching link path %q, got %v", link, got.Files)
	}
	if !strings.Contains(got.Text, "from target") {
		t.Errorf("symlink target contents were not read: %q", got.Text)
	}
}

func TestDirectorySymlinkTraversalIsRecursiveAndCycleSafe(t *testing.T) {
	dir := t.TempDir()
	write := func(path, contents string) {
		t.Helper()
		if err := os.WriteFile(path, []byte(contents), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	top := filepath.Join(dir, "top.ts")
	sub := filepath.Join(dir, "sub")
	inside := filepath.Join(sub, "inside.ts")
	alias := filepath.Join(dir, "alias")
	back := filepath.Join(sub, "back-to-root")
	if err := os.Mkdir(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	write(top, "top")
	write(inside, "inside")
	if err := os.Symlink(sub, alias); err != nil {
		t.Skipf("symlinks are unavailable: %v", err)
	}
	if err := os.Symlink(dir, back); err != nil {
		t.Skipf("directory symlinks are unavailable: %v", err)
	}

	shallow, err := source.Resolve("@dir:"+dir, opts(dir))
	if err != nil {
		t.Fatal(err)
	}
	if len(shallow.Files) != 1 || shallow.Files[0] != top {
		t.Errorf("shallow traversal should skip real and linked directories, got %v", shallow.Files)
	}

	o := opts(dir)
	o.Recursive = true
	deep, err := source.Resolve("@dir:"+dir, o)
	if err != nil {
		t.Fatal(err)
	}
	want := map[string]bool{
		top:                               true,
		inside:                            true,
		filepath.Join(alias, "inside.ts"): true,
	}
	if len(deep.Files) != len(want) {
		t.Fatalf("recursive aliases should be traversed once per path, got %v", deep.Files)
	}
	for _, file := range deep.Files {
		if !want[file] {
			t.Errorf("unexpected file from recursive traversal: %q", file)
		}
	}
}

func TestDirectorySkipsDanglingLinkAndFollowsLinkOutsideRoot(t *testing.T) {
	root := t.TempDir()
	external := t.TempDir()
	regular := filepath.Join(root, "regular.ts")
	dangling := filepath.Join(root, "dangling.ts")
	linkedDir := filepath.Join(root, "external")
	externalFile := filepath.Join(external, "linked.ts")
	if err := os.WriteFile(regular, []byte("regular"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(externalFile, []byte("external"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(filepath.Join(root, "missing.ts"), dangling); err != nil {
		t.Skipf("symlinks are unavailable: %v", err)
	}
	if err := os.Symlink(external, linkedDir); err != nil {
		t.Skipf("directory symlinks are unavailable: %v", err)
	}

	o := opts(root)
	o.Recursive = true
	got, err := source.Resolve("@dir:"+root, o)
	if err != nil {
		t.Fatal(err)
	}
	want := map[string]bool{regular: true, filepath.Join(linkedDir, "linked.ts"): true}
	if len(got.Files) != len(want) {
		t.Fatalf("got files %v, want exactly the regular and external-link paths", got.Files)
	}
	for _, file := range got.Files {
		if !want[file] {
			t.Errorf("unexpected file %q", file)
		}
	}
}

func TestDirectorySkipsSymlinkLoops(t *testing.T) {
	for _, tc := range []struct {
		name  string
		links map[string]string
	}{
		{"self", map[string]string{"loop.ts": "loop.ts"}},
		{"pair", map[string]string{"a.ts": "b.ts", "b.ts": "a.ts"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			root := t.TempDir()
			regular := filepath.Join(root, "regular.ts")
			if err := os.WriteFile(regular, []byte("regular"), 0o644); err != nil {
				t.Fatal(err)
			}
			for name, target := range tc.links {
				if err := os.Symlink(target, filepath.Join(root, name)); err != nil {
					t.Skipf("symlinks are unavailable: %v", err)
				}
			}
			got, err := source.Resolve("@dir:"+root, opts(root))
			if err != nil {
				t.Fatal(err)
			}
			if len(got.Files) != 1 || got.Files[0] != regular {
				t.Fatalf("got files %v, want [%s]", got.Files, regular)
			}
		})
	}
}

func TestResolveAllJoinsInOrder(t *testing.T) {
	dir := t.TempDir()
	_ = os.WriteFile(filepath.Join(dir, "second.ts"), []byte("SECOND"), 0o644)
	got, err := source.ResolveAll([]string{"FIRST", "second.ts", "THIRD"}, opts(dir))
	if err != nil {
		t.Fatal(err)
	}
	if got.Text != "FIRST\nSECOND\nTHIRD" {
		t.Errorf("order not preserved: %q", got.Text)
	}
}

func TestNoneMeansNothingAtAll(t *testing.T) {
	got, err := source.Resolve("none", opts(t.TempDir()))
	if err != nil {
		t.Fatal(err)
	}
	if got.Kind != source.KindNone || got.Text != "" {
		t.Errorf("none should resolve to nothing: %+v", got)
	}
}

func TestTextAndFileAtTheSameLevelIsAConflict(t *testing.T) {
	if err := source.Conflict("script.prefix", "inline", "/a/file.ts"); err == nil {
		t.Fatal("both forms at one level cannot be ordered, so it should be refused")
	}
	if err := source.Conflict("script.prefix", "inline", ""); err != nil {
		t.Errorf("one form alone is fine: %v", err)
	}
}

func TestResolveFileWithTildeNameReadsLiteralDirUnderDir(t *testing.T) {
	// End to end for #2988: @file:~alice/... must read the literal
	// ~alice directory under Dir, never $HOME/alice.
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := os.MkdirAll(filepath.Join(home, "alice"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(home, "alice", "note.txt"), []byte("from home"), 0o644); err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	literal := filepath.Join(dir, "~alice")
	if err := os.MkdirAll(literal, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(literal, "note.txt"), []byte("from literal dir"), 0o644); err != nil {
		t.Fatal(err)
	}
	got, err := source.Resolve("@file:~alice/note.txt", source.Options{Dir: dir})
	if err != nil {
		t.Fatal(err)
	}
	if got.Text != "from literal dir" {
		t.Errorf("Resolve(@file:~alice/note.txt) text = %q, want %q", got.Text, "from literal dir")
	}
	if len(got.Files) != 1 || got.Files[0] != filepath.Join(literal, "note.txt") {
		t.Errorf("Resolve(@file:~alice/note.txt) files = %v, want [%s]", got.Files, filepath.Join(literal, "note.txt"))
	}
}

func TestADirectoryIsConcatenatedInNaturalOrderPastInt64(t *testing.T) {
	// Digit runs that overflow int64 must still decide the order
	// numerically (#3775). Before the fix, naturalLess gave up on such
	// runs and the fragments concatenated in arbitrary or inverted order.
	cases := []struct {
		name  string
		files []string
		want  string
	}{
		{
			name:  "both overflow, differing suffixes",
			files: []string{"f100000000000000000000000a.ts", "f99999999999999999999999z.ts"},
			want:  "f99999999999999999999999z.ts,f100000000000000000000000a.ts",
		},
		{
			name:  "both overflow, no suffix",
			files: []string{"100000000000000000000000.ts", "99999999999999999999999.ts"},
			want:  "99999999999999999999999.ts,100000000000000000000000.ts",
		},
		{
			name:  "one side overflows int64",
			files: []string{"f9999999999999999999.ts", "f8999999999999999999.ts"},
			want:  "f8999999999999999999.ts,f9999999999999999999.ts",
		},
		{
			name:  "leading zeros in overflow run",
			files: []string{"f00000000000000000000000002.ts", "f10.ts"},
			want:  "f00000000000000000000000002.ts,f10.ts",
		},
		{
			name:  "mixed int64 and overflow runs differ from lexical order",
			files: []string{"f10000000000000000000.ts", "f9000000000000000000.ts"},
			want:  "f9000000000000000000.ts,f10000000000000000000.ts",
		},
		{
			name:  "equal length overflow runs compare lexically",
			files: []string{"f20000000000000000000000.ts", "f10000000000000000000000.ts"},
			want:  "f10000000000000000000000.ts,f20000000000000000000000.ts",
		},
		{
			// compareDigitRuns strips leading zeros before comparing by
			// length, so a zero-padded small run must not outrank a shorter
			// run whose value is larger. Without the TrimLeft the 27-digit
			// run "000000000000000000000000002" looks bigger than "10".
			name:  "leading zeros do not change numeric order",
			files: []string{"f00000000000000000000000002.ts", "f10.ts"},
			want:  "f00000000000000000000000002.ts,f10.ts",
		},
		{
			// A zero-padded run longer than int64 compares numerically
			// against a run that still fits int64. Its value (30) is larger
			// while its lexical position is lower than "4", so a lexical
			// fallback would invert the pair.
			name:  "padded overflow run orders numerically beside a fitting run",
			files: []string{"f000000000000000000000000030.ts", "f4.ts"},
			want:  "f4.ts,f000000000000000000000000030.ts",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			for _, n := range tc.files {
				if err := os.WriteFile(filepath.Join(dir, n), []byte("// "+n), 0o644); err != nil {
					t.Fatal(err)
				}
			}
			got, err := source.Resolve(dir, opts(dir))
			if err != nil {
				t.Fatal(err)
			}
			var order []string
			for _, f := range got.Files {
				order = append(order, filepath.Base(f))
			}
			if strings.Join(order, ",") != tc.want {
				t.Errorf("natural order past int64: got %v, want %s", order, tc.want)
			}
		})
	}
}
