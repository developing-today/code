#!/usr/bin/env bash
# Contract tests for check-rust-module-reach.sh (#12232): a tree whose
# every source file is declared passes; a file nothing declares fails
# by name; a declaration that is only text in a comment or a string
# does not count; and the forms rustc accepts (path attributes, inline
# modules, include!, cfg'd and restricted-visibility modules, bin
# targets, a file named after its directory) are all followed.
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-rust-module-reach.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

put() { # put <path> — content on stdin
  mkdir -p "$(dirname "$1")"
  cat >"$1"
}

crate() { # crate <tree> <name> — an empty crate with a manifest
  put "$1/pkgs/demo/$2/Cargo.toml" <<<"[package]"
  mkdir -p "$1/pkgs/demo/$2/src"
}

expect_fail() { # expect_fail <tree> <label> <file that must be named>...
  local tree="$1" label="$2" out
  shift 2
  out="$TMP/out.txt"
  if bash "$SCRIPT" "$tree" pkgs/demo >"$out"; then
    echo "FAIL: $label: expected exit 1"
    cat "$out"
    exit 1
  fi
  for f in "$@"; do
    grep -q "^UNREACHED $f:" "$out" || {
      echo "FAIL: $label: $f was not named"
      cat "$out"
      exit 1
    }
  done
  echo "pass: $label"
}

# --- 1. Every accepted declaration form is followed.
T="$TMP/ok"
crate "$T" a
S="$T/pkgs/demo/a/src"
put "$S/lib.rs" <<'EOF'
//! mod not_a_module;
pub mod plain;
pub(crate) mod restricted;
#[cfg(feature = "x")]
pub mod gated;
mod dir_with_mod_rs;
mod named_after_dir;
#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
mod inline {
    pub mod leaf;
    mod deeper {
        mod bottom;
    }
}
mod r#try;
const TEXT: &str = "mod in_a_string;";
const RAW: &str = r#"mod in_a_raw_string; " still inside"#;
const BRACE: char = '{';
fn lifetime<'a>(x: &'a str) -> &'a str { x }
include!("generated.rs");
EOF
for f in plain restricted gated named_after_dir lib_tests generated try; do put "$S/$f.rs" <<<"// $f"; done
put "$S/dir_with_mod_rs/mod.rs" <<<"pub mod child;"
put "$S/dir_with_mod_rs/child.rs" <<<"// child"
put "$S/named_after_dir.rs" <<<"pub mod inner;"
put "$S/named_after_dir/inner.rs" <<<"// inner"
put "$S/inline/leaf.rs" <<<"// leaf"
put "$S/inline/deeper/bottom.rs" <<<"// bottom"
# A second crate: a binary with its own module beside it, and a bin dir.
crate "$T" b
put "$T/pkgs/demo/b/src/main.rs" <<<"mod cli;"
put "$T/pkgs/demo/b/src/cli.rs" <<<"// cli"
put "$T/pkgs/demo/b/src/bin/tool.rs" <<<"mod helper;"
put "$T/pkgs/demo/b/src/bin/helper.rs" <<<"// helper"
put "$T/pkgs/demo/b/src/bin/multi/main.rs" <<<"mod part;"
put "$T/pkgs/demo/b/src/bin/multi/part.rs" <<<"// part"
# Files outside src/ are not this check's business.
put "$T/pkgs/demo/b/tests/it.rs" <<<"// integration test"
put "$T/pkgs/demo/b/target/debug/build/out.rs" <<<"// build output"
bash "$SCRIPT" "$T" pkgs/demo >"$TMP/ok.out"
grep -q "rust-module-reach: OK (19 files in 2 crates, all reached)" "$TMP/ok.out" || {
  echo "FAIL: compliant tree"
  cat "$TMP/ok.out"
  exit 1
}
echo "pass: every declaration form is followed (19 files, 2 crates)"

# --- 2. A file nothing declares fails, by name. This is the 2026-11
#        shape: the file is merged, a test includes it by path, and the
#        crate root never mentions it.
T="$TMP/orphan"
crate "$T" a
put "$T/pkgs/demo/a/src/lib.rs" <<<"pub mod wired;"
put "$T/pkgs/demo/a/src/wired.rs" <<<"// wired"
put "$T/pkgs/demo/a/src/signed_envelope.rs" <<<"// merged, never declared"
put "$T/pkgs/demo/a/src/graph/mod.rs" <<<"// root that declares no children"
put "$T/pkgs/demo/a/tests/uses_it.rs" <<<'#[path = "../src/signed_envelope.rs"] mod signed_envelope;'
expect_fail "$T" "an undeclared file and an undeclared directory are named" \
  pkgs/demo/a/src/signed_envelope.rs pkgs/demo/a/src/graph/mod.rs
grep -q "2 file(s) are in src/ but not in their crate" "$TMP/out.txt"
echo "pass: the count is reported"

# --- 3. A child of an undeclared parent is unreached too.
T="$TMP/nested"
crate "$T" a
put "$T/pkgs/demo/a/src/lib.rs" <<<"pub mod parent;"
put "$T/pkgs/demo/a/src/parent/mod.rs" <<<"// declares nothing"
put "$T/pkgs/demo/a/src/parent/child.rs" <<<"// never declared"
expect_fail "$T" "a file its parent does not declare is named" pkgs/demo/a/src/parent/child.rs

# --- 4. Text that only looks like a declaration does not count.
T="$TMP/fake"
crate "$T" a
put "$T/pkgs/demo/a/src/lib.rs" <<'EOF'
// mod commented;
/* mod block_commented; /* nested */ mod still_commented; */
/// mod doc_commented;
const S: &str = "mod in_string;";
const R: &str = r##"mod in_raw_string;"##;
EOF
for f in commented block_commented still_commented doc_commented in_string in_raw_string; do
  put "$T/pkgs/demo/a/src/$f.rs" <<<"// $f"
done
expect_fail "$T" "declarations inside comments and strings do not count" \
  pkgs/demo/a/src/commented.rs pkgs/demo/a/src/block_commented.rs \
  pkgs/demo/a/src/still_commented.rs pkgs/demo/a/src/doc_commented.rs \
  pkgs/demo/a/src/in_string.rs pkgs/demo/a/src/in_raw_string.rs

# --- 5. An inline module's children live in its directory, not beside it.
T="$TMP/inline"
crate "$T" a
put "$T/pkgs/demo/a/src/lib.rs" <<'EOF'
mod outer {
    mod leaf;
}
EOF
put "$T/pkgs/demo/a/src/outer/leaf.rs" <<<"// the real one"
put "$T/pkgs/demo/a/src/leaf.rs" <<<"// same name, wrong place"
expect_fail "$T" "an inline module does not reach a same-named file beside it" pkgs/demo/a/src/leaf.rs
if grep -q "UNREACHED pkgs/demo/a/src/outer/leaf.rs" "$TMP/out.txt"; then
  echo "FAIL: the inline module's own child was reported"
  exit 1
fi
echo "pass: the inline module's own child is reached"

# --- 6. A path attribute on an inline module is resolved from the file's
#        directory at the top level and from the enclosing inline module's
#        directory when nested.
T="$TMP/inline_path"
crate "$T" a
put "$T/pkgs/demo/a/src/lib.rs" <<'EOF'
mod outer {
    #[path = "alternate"]
    mod inner {
        mod leaf;
    }
}
#[path = "elsewhere"]
mod top {
    mod leaf;
}
mod from_file;
EOF
put "$T/pkgs/demo/a/src/outer/alternate/leaf.rs" <<<"// reached through outer and the path attribute"
put "$T/pkgs/demo/a/src/elsewhere/leaf.rs" <<<"// reached from the file's directory"
put "$T/pkgs/demo/a/src/from_file.rs" <<'EOF'
#[path = "beside_file"]
mod inline {
    mod leaf;
}
EOF
put "$T/pkgs/demo/a/src/beside_file/leaf.rs" <<<"// a file module's inline path starts at the file's directory"
put "$T/pkgs/demo/a/src/alternate/leaf.rs" <<<"// not where rustc looks for outer::inner::leaf"
expect_fail "$T" "a nested inline path attribute keeps the enclosing directory" pkgs/demo/a/src/alternate/leaf.rs
if grep -q "UNREACHED pkgs/demo/a/src/\(outer\|elsewhere\|beside_file\)/" "$TMP/out.txt"; then
  echo "FAIL: a file rustc compiles was reported"
  cat "$TMP/out.txt"
  exit 1
fi
echo "pass: files reached through inline path attributes are not reported"

# --- 7. A declaration inside a macro_rules! definition is not followed.
T="$TMP/macro"
crate "$T" a
put "$T/pkgs/demo/a/src/lib.rs" <<'EOF'
macro_rules! never_called {
    () => {
        mod in_braces;
    };
}
macro_rules! parens (
    () => { mod in_parens; };
);
pub mod after_the_macros;
EOF
for f in in_braces in_parens after_the_macros; do put "$T/pkgs/demo/a/src/$f.rs" <<<"// $f"; done
expect_fail "$T" "declarations inside macro_rules! definitions do not count" \
  pkgs/demo/a/src/in_braces.rs pkgs/demo/a/src/in_parens.rs
if grep -q "UNREACHED pkgs/demo/a/src/after_the_macros.rs" "$TMP/out.txt"; then
  echo "FAIL: a declaration after a macro definition was lost"
  exit 1
fi
echo "pass: a declaration after the macro definitions is still followed"

# --- 8. Roots come from the manifest.
T="$TMP/manifest"
# [lib] path: the root is not src/lib.rs.
put "$T/pkgs/demo/libpath/Cargo.toml" <<'EOF'
[package]
name = "libpath"
[lib]
path = "src/entry.rs"
EOF
put "$T/pkgs/demo/libpath/src/entry.rs" <<<"mod wired;"
put "$T/pkgs/demo/libpath/src/wired.rs" <<<"// wired"
put "$T/pkgs/demo/libpath/src/orphan.rs" <<<"// never declared"
# autobins = false: only the listed bin is a root.
put "$T/pkgs/demo/nobins/Cargo.toml" <<'EOF'
[package]
name = "nobins"
autobins = false
[[bin]]
name = "listed"
path = "src/bin/listed.rs"
EOF
put "$T/pkgs/demo/nobins/src/lib.rs" <<<"// lib"
put "$T/pkgs/demo/nobins/src/bin/listed.rs" <<<"// a listed bin"
put "$T/pkgs/demo/nobins/src/bin/unlisted.rs" <<<"// not a target while autobins is off"
expect_fail "$T" "roots are read from [lib] path and autobins" \
  pkgs/demo/libpath/src/orphan.rs pkgs/demo/nobins/src/bin/unlisted.rs
for reached in libpath/src/entry.rs libpath/src/wired.rs nobins/src/bin/listed.rs nobins/src/lib.rs; do
  if grep -q "UNREACHED pkgs/demo/$reached" "$TMP/out.txt"; then
    echo "FAIL: $reached was reported"
    exit 1
  fi
done
echo "pass: manifest-declared roots and their modules are reached"

# --- 9. A crate whose root cannot be found fails by name; it is not skipped.
T="$TMP/rootless"
crate "$T" a
put "$T/pkgs/demo/a/src/not_a_root.rs" <<<"// no lib.rs, no main.rs, nothing in the manifest"
set +e
bash "$SCRIPT" "$T" pkgs/demo >"$TMP/rootless.out"
rc=$?
set -e
if [ "$rc" -ne 1 ] || ! grep -q "^NOROOT pkgs/demo/a:" "$TMP/rootless.out"; then
  echo "FAIL: a crate with no root: expected exit 1 and a NOROOT line, got $rc"
  cat "$TMP/rootless.out"
  exit 1
fi
echo "pass: a crate with source files and no root fails by name"

# --- 10. A package directory with no crate is a usage error, not a pass.
mkdir -p "$TMP/empty/pkgs/demo"
set +e
bash "$SCRIPT" "$TMP/empty" pkgs/demo >"$TMP/empty.out"
rc=$?
set -e
[ "$rc" -eq 2 ] || {
  echo "FAIL: empty package: expected exit 2, got $rc"
  exit 1
}
echo "pass: a package with no crate exits 2"

echo "check-rust-module-reach: all contract tests passed"
