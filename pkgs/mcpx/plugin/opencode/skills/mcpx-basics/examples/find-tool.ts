// Find a tool without leaving the script, then print its signature.
//
//   mcpx run find-tool.ts screenshot
//   mcpx run find-tool.ts issue create
//
// search() and describe() read the generated client, so this starts no
// server and costs nothing. Use it instead of `mcpx types` on every
// namespace.
const words = Deno.args.join(" ");
if (!words) {
  console.error("usage: mcpx run find-tool.ts <words...>");
  Deno.exit(2);
}
const hits = search(words, 5);
if (hits.length === 0) {
  console.log(`no tool matches ${JSON.stringify(words)}; namespaces: ${Object.keys(tools).join(", ")}`);
} else {
  // search() returns tools and recipes (CODE-36): a recipe has no
  // namespace/tool to describe, so name it instead.
  for (const h of hits) {
    console.log(h.kind === "recipe" ? `recipe: ${h.name}` : describe(`${h.namespace}.${h.tool}`));
  }
}
