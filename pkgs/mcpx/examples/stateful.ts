// `open` records a value in the server process and `state` reads it back, so
// a later call in the same script can see what an earlier one did.
//
// log() writes to the durable log on stderr; console.log is the script's
// stdout, which is what the expected output below holds.

log("recording two values");

console.log(String(await tools.demo.open({ value: "alpha" })));
console.log(String(await tools.demo.open({ value: "beta" })));

const state = await tools.demo.state();

console.log("alpha recorded:", state.seen.includes("alpha"));
console.log("beta recorded:", state.seen.includes("beta"));
