# The home-page example: copy it or run it in the page

The example on the home page is not a picture of code. It is the working
script from [`home-example/example.ts`](home-example/example.ts), and a
reader can do two things with it before installing anything is required
of them — and a third once it is.

**Copy as mcpx exec.** One click puts a command on the clipboard that
runs the example against your own daemon and your own servers:

```sh
mcpx exec "$(cat <<'MCPX_EOF'
const echoed = await tools.demo.echo({ message: "hello from a script" });

console.log("demo.echo:", String(echoed));
MCPX_EOF
)"
```

The heredoc is quoted, which is the whole trick: an example full of
apostrophes, `$` signs and backticks arrives at `mcpx exec` byte for
byte, because `mcpx exec` takes its source as an argument and its stdin
belongs to the script. *Copy as script* puts the source itself on the
clipboard instead, for a file under `.mcpx/scripts/` and `mcpx run`.

**Run it in the page.** The example is editable, and Run executes the
edited version in the browser tab. This is a simulation and the page
labels it as one: the script runs through the real generated client, but
its `POST /v1/call` is answered by a fake daemon in the page whose only
namespace is a mock `demo` server. It is the right tool for "what does
this code do" — change a line, see the output change — and the wrong one
for "what will this do against my servers", which is what the copy
button is for.

What the simulation does not have: real MCP servers, the launcher, and
the runtime APIs (`Deno.*`, `node:fs`) a real run provides. Running an
example against your own daemon from the page is a later, opt-in stage;
running strangers' code on a hosted daemon is not planned at all. The
component, its mocks, and its regeneration script live in
[`home-example/`](home-example/README.md).
