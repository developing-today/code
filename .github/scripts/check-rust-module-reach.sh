#!/usr/bin/env bash
# check-rust-module-reach.sh — every Rust source file under a crate's
# src/ is compiled into that crate (#12232).
#
# A .rs file that no `mod` declaration reaches is not part of its crate:
# nothing can call it, clippy and rustdoc never see it, and a test that
# includes it with `#[path]` checks a private copy. In 2026-11 the #9106
# substrate sat that way for a week: 44 merged files, 19,477 lines, in
# id-core, id-sync and id-store, because each slice was told not to edit
# a crate root and the slice that owned the roots had already closed.
#
# For each crate under the given package directories this walks the
# module tree from src/lib.rs, src/main.rs and src/bin/*, following
# `mod x;`, `#[path = "..."] mod x;`, inline `mod x { ... }` blocks and
# `include!("...")`, and fails naming every src/**/*.rs it never
# reached. The rule for a new module: declare it in its parent in the
# change that adds the file.
#
# Roots come from the manifest: `[lib] path`, each `[[bin]] path`, and
# the conventional src/lib.rs, src/main.rs and src/bin/* (the last two
# only while `autobins` is not false). A crate with source files and no
# root this check can find fails by name rather than passing unread.
#
# A `mod x;` written inside a `macro_rules!` definition is not followed:
# the definition compiles nothing, and what an invocation expands to
# cannot be read from the text. Declare the module outside the macro.
#
# Usage: check-rust-module-reach.sh [REPO_ROOT [PACKAGE_DIR...]]
#   PACKAGE_DIR defaults to pkgs/id and pkgs/fleet, relative to REPO_ROOT.
# Exit: 0 = every file is reached; 1 = unreached files or a crate with no
#       root (named on stdout); 2 = a package directory holds no crate.
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
shift || true

exec python3 -I - "$ROOT" "$@" <<'PYEOF'
import re
import sys
import tomllib
from pathlib import Path

root = Path(sys.argv[1])
packages = sys.argv[2:] or ["pkgs/id", "pkgs/fleet"]

SKIP_DIRS = {"target", "node_modules", ".git", "result"}


def mask(text):
    """Blank comments and replace each string literal with "\\0N", so the
    scan below never reads code out of a comment or a string. Returns the
    masked text and the literal table."""
    out, strings = [], []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        two = text[i : i + 2]
        if two == "//":
            j = text.find("\n", i)
            i = n if j < 0 else j
        elif two == "/*":
            depth, i = 1, i + 2
            while i < n and depth:
                if text[i : i + 2] == "/*":
                    depth, i = depth + 1, i + 2
                elif text[i : i + 2] == "*/":
                    depth, i = depth - 1, i + 2
                else:
                    if text[i] == "\n":
                        out.append("\n")
                    i += 1
        elif c == '"' or (c in "rb" and re.match(r'(?:br|r)#*"|b"', text[i:])):
            m = re.match(r'(?:br|r)(#*)"', text[i:])
            if m:
                close = '"' + m.group(1)
                start = i + m.end()
                j = text.find(close, start)
                j = n if j < 0 else j
                body, i = text[start:j], j + len(close)
            else:
                start = i + (2 if c == "b" else 1)
                j = start
                while j < n and text[j] != '"':
                    j += 2 if text[j] == "\\" else 1
                body, i = text[start:j], j + 1
            out.append('"\0%d"' % len(strings))
            out.append("\n" * body.count("\n"))
            strings.append(body)
        elif c == "'":
            m = re.match(r"'(?:\\.[^']*|[^\\'])'", text[i:])
            if m:
                out.append("' '")
                i += m.end()
            else:
                out.append(c)
                i += 1
        else:
            out.append(c)
            i += 1
    return "".join(out), strings


MACRO_DEF = re.compile(r"\bmacro_rules\s*!\s*[A-Za-z_][A-Za-z0-9_]*\s*([({\[])")
CLOSER = {"(": ")", "{": "}", "[": "]"}


def drop_macro_definitions(text):
    """Blank the body of every `macro_rules!` definition (newlines kept).
    Comments and strings are already masked, so delimiters balance."""
    out, i = [], 0
    while True:
        m = MACRO_DEF.search(text, i)
        if not m:
            out.append(text[i:])
            return "".join(out)
        opener = m.group(1)
        depth, j = 1, m.end()
        while j < len(text) and depth:
            if text[j] == opener:
                depth += 1
            elif text[j] == CLOSER[opener]:
                depth -= 1
            j += 1
        out.append(text[i : m.start()])
        out.append("\n" * text.count("\n", m.start(), j))
        i = j


TOKEN = re.compile(
    r"""
    (?P<attrs>(?:\#\s*!?\[(?:[^\[\]]|\[[^\[\]]*\])*\]\s*)*)
    (?:pub(?:\s*\([^)]*\))?\s+)?
    mod\s+(?P<name>(?:r\#)?[A-Za-z_][A-Za-z0-9_]*)\s*(?P<end>[;{])
    | (?P<include>\binclude!\s*[(\[{]\s*"\0(?P<inc>\d+)"\s*[)\]}])
    | (?P<open>\{)
    | (?P<close>\})
    """,
    re.X,
)
PATH_ATTR = re.compile(r'#\s*\[\s*path\s*=\s*"\0(\d+)"\s*\]')


def children_dir(path):
    """Where `mod x;` in this file looks for x.rs."""
    if path.name in ("mod.rs", "lib.rs", "main.rs"):
        return path.parent
    return path.parent / path.stem


def walk(path, reached, missing, top=False):
    path = path.resolve()
    if path in reached:
        return
    reached.add(path)
    try:
        text, strings = mask(path.read_text(encoding="utf-8", errors="replace"))
    except OSError:
        return
    text = drop_macro_definitions(text)
    # A bin target at src/bin/x.rs is a crate root: its modules sit beside it.
    base = path.parent if top else children_dir(path)
    stack = []  # inline modules: (directory before entering, brace depth)
    depth = 0
    for m in TOKEN.finditer(text):
        if m.group("open"):
            depth += 1
        elif m.group("close"):
            depth -= 1
            if stack and stack[-1][1] == depth:
                base = stack.pop()[0]
        elif m.group("include"):
            target = path.parent / strings[int(m.group("inc"))]
            if target.suffix == ".rs" and target.exists():
                walk(target, reached, missing)
        else:
            name = m.group("name").removeprefix("r#")
            attr = PATH_ATTR.search(m.group("attrs") or "")
            # Rust resolves a path attribute from the file's own
            # directory, or, inside an inline module, from that inline
            # module's directory.
            origin = base if stack else path.parent
            if m.group("end") == "{":
                stack.append((base, depth))
                depth += 1
                base = (origin / strings[int(attr.group(1))]) if attr else base / name
                continue
            if attr:
                candidates = [origin / strings[int(attr.group(1))]]
            else:
                candidates = [base / (name + ".rs"), base / name / "mod.rs"]
            found = next((c for c in candidates if c.exists()), None)
            if found is None:
                # A declaration whose file is absent (a cfg'd-out or
                # generated module). rustc reports that; this check is
                # about files, not declarations.
                missing.append((path, name))
            else:
                walk(found, reached, missing)


def crates(package):
    """Every directory under `package` holding Cargo.toml and src/."""
    found = []
    stack = [package]
    while stack:
        d = stack.pop()
        if (d / "Cargo.toml").exists() and (d / "src").is_dir():
            found.append(d)
        for child in sorted(d.iterdir()):
            if child.is_dir() and child.name not in SKIP_DIRS and not child.name.startswith("."):
                if child.name in ("src", "tests", "benches", "examples"):
                    continue
                stack.append(child)
    return sorted(found)


def crate_roots(crate):
    """The files rustc starts from for this crate's src/ targets."""
    src = crate / "src"
    try:
        manifest = tomllib.loads((crate / "Cargo.toml").read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError):
        manifest = {}
    package = manifest.get("package") if isinstance(manifest.get("package"), dict) else {}
    roots = []
    lib = manifest.get("lib") if isinstance(manifest.get("lib"), dict) else {}
    lib_path = crate / lib["path"] if isinstance(lib.get("path"), str) else src / "lib.rs"
    if lib_path.exists():
        roots.append(lib_path)
    for kind in ("bin", "example", "bench", "test"):
        for target in manifest.get(kind) or []:
            if isinstance(target, dict) and isinstance(target.get("path"), str):
                declared = crate / target["path"]
                if declared.exists():
                    roots.append(declared)
    if package.get("autobins", True) is not False:
        if (src / "main.rs").exists():
            roots.append(src / "main.rs")
        bindir = src / "bin"
        if bindir.is_dir():
            roots += sorted(bindir.glob("*.rs")) + sorted(bindir.glob("*/main.rs"))
    # Only roots inside src/ start a walk of src/.
    return [r for r in roots if src.resolve() in r.resolve().parents]


problems = []
rootless = []
total_files = 0
total_crates = 0
for package in packages:
    pdir = root / package
    found = crates(pdir) if pdir.is_dir() else []
    if not found:
        print(f"rust-module-reach: no crate under {package}")
        sys.exit(2)
    for crate in found:
        src = crate / "src"
        roots = crate_roots(crate)
        if not roots:
            if any(src.rglob("*.rs")):
                rootless.append(crate.relative_to(root).as_posix())
            continue
        total_crates += 1
        reached, missing = set(), []
        for r in roots:
            walk(r, reached, missing, top=True)
        for f in sorted(src.rglob("*.rs")):
            total_files += 1
            if f.resolve() not in reached:
                problems.append(f.relative_to(root).as_posix())

for crate in rootless:
    print(
        f"NOROOT {crate}: source files under src/ but no crate root found "
        "(src/lib.rs, src/main.rs, src/bin/*, or a [lib] or [[bin]] path in Cargo.toml)"
    )
if problems:
    for p in problems:
        print(f"UNREACHED {p}: no `mod` declaration compiles this file into its crate")
    print(
        f"rust-module-reach: {len(problems)} file(s) are in src/ but not in their crate. "
        "Declare each in its parent module (lib.rs, mod.rs or the file named after its directory) "
        "in the change that adds it, or move test-only code under tests/."
    )
if problems or rootless:
    sys.exit(1)
print(f"rust-module-reach: OK ({total_files} files in {total_crates} crates, all reached)")
PYEOF
