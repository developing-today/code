#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 0 ]]; then
  echo "Usage: SITE_PROJECT_ID=<id> bash scripts/build.sh" >&2
  exit 2
fi
cd "$(dirname "${BASH_SOURCE[0]}")/.."
node --input-type=module - <<'JS'
import fs from "node:fs";
const id = process.env.SITE_PROJECT_ID || "";
if (!/^[a-zA-Z0-9_-]{8,128}$/.test(id)) throw Error("Set SITE_PROJECT_ID to the target private Site project ID");
const tracked = ".openai/hosting.json";
const manifest = fs.existsSync(tracked) ? JSON.parse(fs.readFileSync(tracked, "utf8")) :
  {project_id:id,d1:null,r2:"BUCKET",capabilities:["mcp"]};
if (manifest.project_id!==id || manifest.static || manifest.r2!=="BUCKET" ||
    !Array.isArray(manifest.capabilities) || !manifest.capabilities.includes("mcp"))
  throw Error("Target identity, Worker, BUCKET and MCP capability must match");
function checkExisting(path, expected) {
  let st;
  try { st = fs.lstatSync(path); } catch (e) { if (e.code === "ENOENT") return; throw e; }
  if (expected === null) {
    if (!st.isFile() || st.nlink !== 1) throw Error("Unexpected artifact file: " + path);
    return;
  }
  if (!st.isDirectory()) throw Error("Unexpected artifact directory: " + path);
  for (const name of fs.readdirSync(path)) {
    if (!Object.hasOwn(expected, name)) throw Error("Unexpected artifact entry: " + path + "/" + name);
    checkExisting(path + "/" + name, expected[name]);
  }
}
checkExisting("dist", {server:{"index.js":null}, ".openai":{"hosting.json":null}});
fs.mkdirSync("dist/server",{recursive:true});
fs.mkdirSync("dist/.openai",{recursive:true});
fs.copyFileSync("worker/index.js","dist/server/index.js");
fs.writeFileSync("dist/.openai/hosting.json",JSON.stringify(manifest,null,2)+"\n");
JS
