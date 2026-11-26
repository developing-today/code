// #7983 item 6: BundleFlight.fetch read the posted job and used its
// fields (in-flight key, Drive URL path, Range header) with no shape
// check. A malformed job must get the module's 400 error vocabulary
// before any token exchange or Drive fetch happens.
//
// Run: node test/job-body.test.mjs   (also wired into `npm test`)

import { BundleFlight, parseJob } from "../src/index.js";

let failures = 0;
function check(name, cond, detail = "") {
  if (cond) {
    console.log(`ok: ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL: ${name} ${detail}`);
  }
}

const good = {
  sha256: "a".repeat(64),
  bundle_id: "b_0123456789abcdef",
  drive_file_id: "1AbC-d_E",
  offset: 0,
  frame_length: 10,
  size: 5,
};

let fetches = 0;
const realFetch = globalThis.fetch;
globalThis.fetch = async () => {
  fetches += 1;
  throw new Error("network disabled in this test");
};

const flight = new BundleFlight(
  { storage: { async get() { return null; }, async put() {} } },
  { GOOGLE_CLIENT_ID: "id", GOOGLE_CLIENT_SECRET: "secret", GOOGLE_REFRESH_TOKEN: "rt" },
);
const post = (body) =>
  flight.fetch(new Request("https://bundle-flight/rehydrate", { method: "POST", body }));

const probes = [
  ["not json", "{"],
  ["json null", "null"],
  ["json array", "[]"],
  ["json scalar", "7"],
  ["empty object", "{}"],
  ["sha256 wrong type", { ...good, sha256: 1 }],
  ["sha256 not hex", { ...good, sha256: "z".repeat(64) }],
  ["bundle_id empty", { ...good, bundle_id: "" }],
  ["drive_file_id wrong type", { ...good, drive_file_id: 5 }],
  ["drive_file_id with a path", { ...good, drive_file_id: "../../other?alt=json" }],
  ["offset negative", { ...good, offset: -1 }],
  ["offset string", { ...good, offset: "0" }],
  ["frame_length zero", { ...good, frame_length: 0 }],
  ["frame_length fractional", { ...good, frame_length: 1.5 }],
];

try {
  for (const [name, body] of probes) {
    const resp = await post(typeof body === "string" ? body : JSON.stringify(body));
    const json = await resp.json();
    check(`${name}: 400 with the module's error envelope`,
      resp.status === 400 && typeof json.error === "string", `got ${resp.status} ${JSON.stringify(json)}`);
  }
  check("no malformed job reached a token exchange or Drive fetch", fetches === 0, `fetches=${fetches}`);

  check("parseJob accepts a manifest row", parseJob(good).job === good);

  // Control: a well-formed job gets past the check and tries the network.
  const resp = await post(JSON.stringify(good));
  check("control: a well-formed job reaches the token exchange", fetches === 1 && resp.status === 502,
    `fetches=${fetches} status=${resp.status}`);
} finally {
  globalThis.fetch = realFetch;
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall job-body checks passed");
