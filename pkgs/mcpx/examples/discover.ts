// Find out what a namespace offers from inside the script, then call two of
// the tools it lists.
//
// `search()` and `describe()` read the generated client, so they cost no
// round trip to the server.

console.log("tools:", Object.keys(tools.demo).join(", "));
console.log(describe("demo.echo").trim());

const echoed = await tools.demo.echo({ message: "two tools" });
const structured = await tools.demo.structured();

console.log("echo:", String(echoed));
console.log("structured:", JSON.stringify(structured));
