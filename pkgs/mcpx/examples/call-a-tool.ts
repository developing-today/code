// Call one tool and print what it returns.
//
// Nothing is imported: `tools` is already on globalThis.

const echoed = await tools.demo.echo({ message: "hello from a script" });

console.log("demo.echo:", String(echoed));
