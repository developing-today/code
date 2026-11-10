// Stage 1 of issue #1876 Approach C: copy-as-command.
//
// Two clipboard payloads for a docs example:
//
//   execCommand(source)          -> one line that runs the snippet with
//                                   `mcpx exec` against the reader's real
//                                   daemon and real servers.
//   scriptCommands(name, source) -> two lines that save the snippet as a
//                                   named script and run it with `mcpx run`.
//
// The one trap (named in the issue): shell quoting. `mcpx exec` takes the
// source as an argument and reads nothing from stdin (stdin is passed
// through to the script), so the source has to arrive inside the command
// itself. Single quotes cannot contain an apostrophe, so the source is
// wrapped in a *quoted* heredoc inside a command substitution:
//
//   mcpx exec "$(cat <<'MCPX_EOF'
//   <source, verbatim — apostrophes, $, backticks all inert>
//   MCPX_EOF
//   )"
//
// Quoting the delimiter ('MCPX_EOF') is what turns expansion off inside the
// body. The delimiter is extended until no line of the source equals it, so
// a source that itself mentions MCPX_EOF still round-trips. Command
// substitution strips trailing newlines; that is fine for source text.
//
// Pure functions, no DOM: the page wires them to buttons, and
// smoke-test.mjs round-trips them through a real shell.

// wrapSource returns the heredoc body form shared by both commands:
// <<'<DELIM>'\n<source>\n<DELIM>, with DELIM chosen clear of the source.
function heredoc(source) {
  let delim = "MCPX_EOF";
  const lines = source.split("\n");
  while (lines.includes(delim)) delim += "_X";
  const body = source.endsWith("\n") ? source : source + "\n";
  return { delim, body };
}

export function execCommand(source) {
  const { delim, body } = heredoc(source);
  return `mcpx exec "$(cat <<'${delim}'\n${body}${delim}\n)"`;
}

const SCRIPT_NAME = /^[a-z0-9][a-z0-9-]*$/;

export function scriptCommands(name, source) {
  if (!SCRIPT_NAME.test(name)) {
    throw new Error(
      `script name ${JSON.stringify(name)} must be lowercase letters, digits and dashes`,
    );
  }
  const { delim, body } = heredoc(source);
  const path = `.mcpx/scripts/${name}.ts`;
  return [
    `mkdir -p .mcpx/scripts && cat > ${path} <<'${delim}'`,
    `${body}${delim}`,
    `mcpx run ${name}`,
  ].join("\n");
}

// asScript is the "Copy as script" payload: the source itself, unchanged —
// the reader pastes it into a file (or their editor) as-is.
export function asScript(source) {
  return source;
}
