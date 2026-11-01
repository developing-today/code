package config

import (
	"crypto/sha256"
	"encoding/hex"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport"
)

func writeExec(t *testing.T, path, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(body), 0o755); err != nil {
		t.Fatal(err)
	}
}

func TestExecIdentityFollowsTheExecutableNotTheCommandName(t *testing.T) {
	// testsupport.TempDir, not t.TempDir: ExecStamp reports the path with
	// symlinks resolved, and on macOS t.TempDir is under /var, a symlink to
	// /private/var, so the unresolved dir never equals the stamp's path (#282).
	dir := testsupport.TempDir(t)
	bin := filepath.Join(dir, "bin", "tool")
	writeExec(t, bin, "#!/bin/sh\necho one\n")
	r := &Resolved{Server: &Server{Command: "tool", Env: map[string]string{"PATH": filepath.Join(dir, "bin")}}}

	first := r.ExecIdentity()
	// #1532: the file is a script (#!), so its identity is the framed
	// hash — content || NUL || "mcpx:exec-dir" || NUL || resolved
	// directory — not the bare content hash this test pinned before.
	h := sha256.New()
	h.Write([]byte("#!/bin/sh\necho one\n"))
	h.Write([]byte{0})
	h.Write([]byte("mcpx:exec-dir"))
	h.Write([]byte{0})
	h.Write([]byte(filepath.Join(dir, "bin")))
	sum := h.Sum(nil)
	if want := "sha256:" + hex.EncodeToString(sum)[:16]; first != want {
		t.Fatalf("identity %q of a small script is not its framed hash %q", first, want)
	}
	if st, ok := r.ExecStamp(); !ok || st.Path != bin {
		t.Fatalf("stamp %+v does not name the resolved file %s", st, bin)
	}
	if again := r.ExecIdentity(); again != first {
		t.Fatalf("identity moved with nothing changed: %q then %q", first, again)
	}
	writeExec(t, bin, "#!/bin/sh\necho two\n")
	if changed := r.ExecIdentity(); changed == first {
		t.Fatal("a rewritten executable kept its identity")
	}
}

func TestExecIdentityResolvesAgainstTheServersOwnPath(t *testing.T) {
	dir := testsupport.TempDir(t)
	writeExec(t, filepath.Join(dir, "elsewhere", "tool"), "#!/bin/sh\n")
	r := &Resolved{Server: &Server{Command: "tool", Env: map[string]string{"PATH": filepath.Join(dir, "elsewhere")}}}
	if st, ok := r.ExecStamp(); !ok || st.Path != filepath.Join(dir, "elsewhere", "tool") {
		t.Fatalf("stamp %+v ignores the PATH the server is given", st)
	}
}

// A symlink re-pointed at a byte-identical file runs the same program, so it
// keeps its identity; the stamp still moves, which is what makes the check
// look again.
func TestExecIdentityOfASmallFileIsItsContentNotItsPath(t *testing.T) {
	dir := t.TempDir()
	first := filepath.Join(dir, "first")
	second := filepath.Join(dir, "second")
	third := filepath.Join(dir, "third")
	writeExec(t, first, "#!/bin/sh\necho same\n")
	writeExec(t, second, "#!/bin/sh\necho same\n")
	writeExec(t, third, "#!/bin/sh\necho other\n")
	link := filepath.Join(dir, "tool")
	if err := os.Symlink(first, link); err != nil {
		t.Fatal(err)
	}
	r := &Resolved{Server: &Server{Command: link}}
	before, _ := r.ExecStamp()
	id := r.ExecIdentity()

	repoint := func(to string) {
		t.Helper()
		if err := os.Remove(link); err != nil {
			t.Fatal(err)
		}
		if err := os.Symlink(to, link); err != nil {
			t.Fatal(err)
		}
	}
	repoint(second)
	if after, _ := r.ExecStamp(); after == before {
		t.Fatal("re-pointing the link did not move the stamp")
	}
	if got := r.ExecIdentity(); got != id {
		t.Fatalf("a link re-pointed at identical bytes changed identity: %q -> %q", id, got)
	}
	repoint(third)
	if got := r.ExecIdentity(); got == id {
		t.Fatal("a link re-pointed at different bytes kept its identity")
	}
}

// A file above the hashing limit is told apart by its inode, not its bytes:
// two files with the same content are still different programs to the pool.
func TestExecIdentityOfALargeFileIsItsInode(t *testing.T) {
	old := defaults.ExecHashMax
	defaults.ExecHashMax = 4
	t.Cleanup(func() { defaults.ExecHashMax = old })

	dir := t.TempDir()
	bin := filepath.Join(dir, "tool")
	writeExec(t, bin, "#!/bin/sh\necho same\n")
	r := &Resolved{Server: &Server{Command: bin}}
	id := r.ExecIdentity()
	if !strings.HasPrefix(id, "stat:") {
		t.Fatalf("identity %q of a file over the hash limit is not a stat identity", id)
	}
	if again := r.ExecIdentity(); again != id {
		t.Fatalf("stat identity moved with nothing changed: %q then %q", id, again)
	}

	// A second file with the same bytes is a different program: the identity
	// is the file, not what it holds, above the limit.
	twin := filepath.Join(dir, "twin")
	writeExec(t, twin, "#!/bin/sh\necho same\n")
	if got := (&Resolved{Server: &Server{Command: twin}}).ExecIdentity(); got == id {
		t.Fatal("a large file with the same bytes as another kept its identity")
	}

	// A replacement at the same path with the same bytes is a new inode.
	tmp := filepath.Join(dir, "tool.new")
	writeExec(t, tmp, "#!/bin/sh\necho same\n")
	if err := os.Rename(tmp, bin); err != nil {
		t.Fatal(err)
	}
	if got := r.ExecIdentity(); got == id {
		t.Fatal("a replaced large file kept its identity")
	}
}

func TestExecIdentityOfAMissingCommandIsItsOwnIdentity(t *testing.T) {
	r := &Resolved{Server: &Server{Command: "no-such-mcpx-tool-anywhere", Env: map[string]string{"PATH": t.TempDir()}}}
	if id := r.ExecIdentity(); !strings.HasPrefix(id, "missing:") {
		t.Fatalf("identity %q for a command that does not resolve", id)
	}
}

func TestAServerThatIsNotAChildHasNoExecIdentity(t *testing.T) {
	r := &Resolved{Server: &Server{URL: "https://example.invalid/mcp"}}
	if id := r.ExecIdentity(); id != "" {
		t.Fatalf("a remote server has identity %q", id)
	}
	if r.ProcessID() != r.PoolID()+"/" {
		t.Fatalf("ProcessID %q is not PoolID plus the identity", r.ProcessID())
	}
}

// #1532: a script's identity folds in the resolved directory of the real
// file. Identical script bytes in different directories are different
// programs; two names in one directory are the same program.
func TestExecIdentityOfAScriptFoldsInItsDirectory(t *testing.T) {
	dir := testsupport.TempDir(t)
	body := "#!/bin/sh\necho same\n"
	a := filepath.Join(dir, "a", "tool")
	b := filepath.Join(dir, "b", "tool")
	writeExec(t, a, body)
	writeExec(t, b, body)
	idA := (&Resolved{Server: &Server{Command: a}}).ExecIdentity()
	if idB := (&Resolved{Server: &Server{Command: b}}).ExecIdentity(); idB == idA {
		t.Fatalf("identical script bytes in different directories kept identity %q", idA)
	}

	// Same directory, another name: the same identity.
	a2 := filepath.Join(dir, "a", "tool2")
	writeExec(t, a2, body)
	if got := (&Resolved{Server: &Server{Command: a2}}).ExecIdentity(); got != idA {
		t.Fatalf("identical script bytes in the same directory changed identity: %q -> %q", idA, got)
	}

	// A symlink re-pointed across directories moves; within one, it keeps.
	link := filepath.Join(dir, "tool")
	if err := os.Symlink(a, link); err != nil {
		t.Fatal(err)
	}
	r := &Resolved{Server: &Server{Command: link}}
	if got := r.ExecIdentity(); got != idA {
		t.Fatalf("a link at the script in its own directory has identity %q, want %q", got, idA)
	}
	repoint := func(to string) {
		t.Helper()
		if err := os.Remove(link); err != nil {
			t.Fatal(err)
		}
		if err := os.Symlink(to, link); err != nil {
			t.Fatal(err)
		}
	}
	repoint(b)
	if got := r.ExecIdentity(); got == idA {
		t.Fatal("a link re-pointed at an identical script in another directory kept its identity")
	}
	repoint(a2)
	if got := r.ExecIdentity(); got != idA {
		t.Fatalf("a link re-pointed within the directory changed identity: %q -> %q", idA, got)
	}
}

// #1532: a non-script file stays bytes-only — identical bytes in different
// directories keep one identity, and it is exactly today's bare content
// hash, so existing binary identities do not move on rollout.
func TestExecIdentityOfANonScriptIgnoresItsDirectory(t *testing.T) {
	dir := testsupport.TempDir(t)
	body := "echo same\n"
	a := filepath.Join(dir, "a", "tool")
	b := filepath.Join(dir, "b", "tool")
	writeExec(t, a, body)
	writeExec(t, b, body)
	sum := sha256.Sum256([]byte(body))
	want := "sha256:" + hex.EncodeToString(sum[:])[:16]
	for _, path := range []string{a, b} {
		if got := (&Resolved{Server: &Server{Command: path}}).ExecIdentity(); got != want {
			t.Fatalf("non-script identity %q at %s is not the bare content hash %q", got, path, want)
		}
	}
}

// #1532: scriptness is a property of the bytes (the #! prefix), so the two
// branches are compared on one byte string: hashed as a script the bytes
// take the framed hash; the bare content hash is what the same bytes would
// identify as without the fold — and a real non-script file's identity is
// exactly its bare hash (above), so a script and a non-script are told
// apart even before their directories enter into it.
func TestExecIdentityOfAScriptIsNotItsBareContentHash(t *testing.T) {
	dir := testsupport.TempDir(t)
	body := "#!/bin/sh\necho same\n"
	bin := filepath.Join(dir, "tool")
	writeExec(t, bin, body)
	sum := sha256.Sum256([]byte(body))
	bare := "sha256:" + hex.EncodeToString(sum[:])[:16]
	if got := (&Resolved{Server: &Server{Command: bin}}).ExecIdentity(); got == bare {
		t.Fatalf("script identity %q is its bare content hash; the directory fold did not apply", got)
	}
}

// #1532: the framing cannot alias. A script whose content embeds the
// marker segment plus another directory's path — the strongest boundary
// shift available, since a real directory cannot contain NUL — still
// hashes as its own (content, directory) pair, not as the plain script
// sitting in that other directory.
func TestExecIdentityScriptFramingDoesNotAlias(t *testing.T) {
	dir := testsupport.TempDir(t)
	other := filepath.Join(dir, "other")
	plain := "#!/bin/sh\necho same\n"
	writeExec(t, filepath.Join(other, "tool"), plain)
	idOther := (&Resolved{Server: &Server{Command: filepath.Join(other, "tool")}}).ExecIdentity()

	tricky := "#!/bin/sh\necho \x00mcpx:exec-dir\x00" + other + "\n"
	mine := filepath.Join(dir, "mine")
	writeExec(t, filepath.Join(mine, "tool"), tricky)
	got := (&Resolved{Server: &Server{Command: filepath.Join(mine, "tool")}}).ExecIdentity()
	if got == idOther {
		t.Fatalf("a script embedding the framing marker aliased the plain script in %s: %q", other, got)
	}
	// And its identity is exactly its own framed hash.
	h := sha256.New()
	h.Write([]byte(tricky))
	h.Write([]byte{0})
	h.Write([]byte("mcpx:exec-dir"))
	h.Write([]byte{0})
	h.Write([]byte(mine))
	sum := h.Sum(nil)
	if want := "sha256:" + hex.EncodeToString(sum)[:16]; got != want {
		t.Fatalf("identity %q is not the framed hash %q of its own (content, directory)", got, want)
	}
}
