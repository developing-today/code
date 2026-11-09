package artifacts

import (
	"fmt"
	"mime"
	"os"
	"path/filepath"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// SanitizeName makes a script-supplied name safe to write to disk.
//
// The name comes from the script, and a script is not trusted with a path.
// The three ways a name escapes a directory are an absolute path, a ..
// component, and a separator that turns one name into a nested path; all
// three are removed rather than rejected, because a caller whose artifact is
// refused over its name has lost the file, and the name is decoration.
//
// Everything outside a conservative set becomes an underscore. That is wider
// than strictly necessary on a modern filesystem and narrower than what a
// shell will mishandle, and the file is reached by id anyway -- the name only
// has to be recognisable.
func SanitizeName(name string) string {
	name = strings.TrimSpace(name)
	// Take the last component: "a/b/../c.png" is a path, and the file is
	// called c.png.
	if i := strings.LastIndexAny(name, `/\`); i >= 0 {
		name = name[i+1:]
	}
	var b strings.Builder
	for _, r := range name {
		switch {
		case r >= 'a' && r <= 'z', r >= 'A' && r <= 'Z', r >= '0' && r <= '9':
			b.WriteRune(r)
		case r == '.' || r == '-' || r == '_' || r == '+':
			b.WriteRune(r)
		case r == ' ':
			b.WriteRune('-')
		default:
			b.WriteRune('_')
		}
	}
	out := strings.Trim(b.String(), ".")
	// "..", "." and "" all collapse to nothing useful above, and a file has
	// to be called something.
	if out == "" {
		return "artifact"
	}
	// A leading dot would hide the file from an ordinary listing, which is
	// not what a caller asking for `.env` in an output directory expects to
	// have to remember.
	if len(out) > defaults.ArtifactNameMaxLength {
		ext := filepath.Ext(out)
		if len(ext) > defaults.ArtifactNameMaxLength/2 {
			ext = ""
		}
		out = out[:defaults.ArtifactNameMaxLength-len(ext)] + ext
	}
	return out
}

// MimeForName guesses a content type from an extension.
//
// A guess, and stated as one: the script may pass the type explicitly and
// that always wins. The fallback is the universal "some bytes" type rather
// than text, because treating an unknown binary as text is how a terminal
// gets filled with control characters.
func MimeForName(name string) string {
	if t := mime.TypeByExtension(strings.ToLower(filepath.Ext(name))); t != "" {
		return t
	}
	return "application/octet-stream"
}

// ClaimPath resolves a name inside a directory without ever overwriting,
// for callers outside this package that deliver artifacts into a directory
// the caller named (the CLI's exec artifact delivery, #1605). It is the
// exported form of claimPath, so every delivery path follows the same
// never-overwrite rule Store.Export documents.
func ClaimPath(dir, name string) (string, *os.File, error) {
	return claimPath(dir, name)
}

// claimPath resolves a name inside a directory without ever overwriting,
// and returns the destination already created, with the claim held open.
//
// The O_EXCL create is the check and the claim in one step, and -- unlike
// the previous resolve-then-rename composition (#4253) -- the claim is not
// released before the write: the caller writes the delivery into the
// returned file and closes it. A name handed out here cannot be handed
// out again or taken by a file that appears later, because it is already
// taken. On any failure after claiming, the caller closes the file and
// removes the path, so a failed delivery leaves nothing that looks
// finished.
//
// The suffix goes before the extension -- shot-1.png, not shot.png-1 -- so
// the file still opens in whatever the extension implies.
func claimPath(dir, name string) (string, *os.File, error) {
	name = SanitizeName(name)
	// Resolved so that a symlinked output directory is still written inside
	// itself. Checked after joining, because the check has to be on the path
	// that will actually be opened.
	base := filepath.Clean(dir)
	ext := filepath.Ext(name)
	stem := strings.TrimSuffix(name, ext)
	for i := 0; i < defaults.ArtifactNameCollisionLimit; i++ {
		candidate := name
		if i > 0 {
			candidate = fmt.Sprintf("%s-%d%s", stem, i, ext)
		}
		p := filepath.Join(base, candidate)
		// Containment via Rel, not a prefix test: Join cleans a "." (or
		// "./") directory away, so "." + "shot.png" joins to the bare
		// "shot.png", which a "."+separator prefix test wrongly rejects
		// even though the file lands in the current directory (Codex
		// finding on #2950). Rel expresses the real question -- is the
		// destination at or below the directory -- for every base,
		// including "." and the filesystem root.
		rel, rerr := filepath.Rel(base, p)
		if rerr != nil || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
			return "", nil, fmt.Errorf("artifact name %q does not stay inside %s", name, dir)
		}
		// Testing with Stat and then creating would be a race two
		// concurrent runs of the same script would lose; the create
		// itself is the test, and the open file is the held claim.
		f, err := os.OpenFile(p, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
		if err == nil {
			return p, f, nil
		}
		if !os.IsExist(err) {
			return "", nil, err
		}
	}
	return "", nil, fmt.Errorf("%s already holds %d files called %q", dir, defaults.ArtifactNameCollisionLimit, name)
}
