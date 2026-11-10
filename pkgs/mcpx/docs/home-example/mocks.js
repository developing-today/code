// The in-page mock namespaces for the home-page simulation (#1876 stage 2).
//
// Honest scope: the only namespace is `demo`, the deterministic fixture
// server from internal/testsupport/fakemcp, and only its deterministic
// tools are mirrored here — echo, open, state, wipe, structured, boom,
// fancy-name — with the same results the fixture returns (state is
// per-page-load here; the fixture's pid is reported as 0 because a browser
// tab has no pid worth printing). Tools whose behaviour is timing,
// notifications, or schema-version dependent (slow, chatty, summarise,
// create_issue, outage) are NOT imitated: the fake daemon answers them
// with an error that says they are outside the simulation, rather than a
// made-up result that teaches the reader something false.
//
// Call shape mirrors the daemon's /v1/call result: a returned object is
// `{ result: <MCP CallToolResult> }`; `{ error: string }` is a transport-
// level failure, which the generated client surfaces as a ToolError.

const state = [];

function text(t) {
  return { content: [{ type: "text", text: t }] };
}

export function callMock(server, tool, args = {}) {
  if (server !== "demo") {
    return {
      error:
        `the simulation only has the mock "demo" namespace; ` +
        `"${server}" is not available in the browser demo`,
    };
  }
  switch (tool) {
    case "echo":
      return { result: text(String(args.message ?? "")) };
    case "open": {
      const v = String(args.value ?? "");
      state.push(v);
      return { result: text("opened " + v) };
    }
    case "state":
      return {
        result: text(JSON.stringify({ pid: 0, seen: [...state] })),
      };
    case "wipe":
      state.length = 0;
      return { result: text("wiped") };
    case "structured":
      // The fixture's every-kind-of-block probe, unchanged.
      return {
        result: {
          content: [
            { type: "text", text: `{"n":42}` },
            { type: "image", data: "iVBORw0KGgo=", mimeType: "image/png" },
            {
              type: "resource_link",
              uri: "demo://greeting",
              name: "greeting",
              mimeType: "image/png",
            },
          ],
          structuredContent: { n: 42 },
          _meta: { "example.com/k": "result" },
        },
      };
    case "boom":
      return {
        result: {
          isError: true,
          content: [{ type: "text", text: "boom: deliberate failure" }],
        },
      };
    case "fancy-name":
      return { result: text(JSON.stringify(args)) };
    default:
      return {
        error:
          `demo.${tool} is not part of the browser simulation; ` +
          `run the example with "Copy as mcpx exec" to try it against a real daemon`,
      };
  }
}
