// d1-gateway: Turso/libSQL-compatible HTTP endpoint (Hrana v2/v3 JSON
// pipelines) over Cloudflare D1, so clients written for Turso move to
// D1 by changing their URL.
//
//   POST /db/<name>/v2/pipeline   (and /v3/pipeline)
//   GET  /db/<name>/v2            version probe (200)
//   GET  /db/<name>/v3            404, so @libsql/client stays on v2
//                                 (v3 cursors are not served)
//   GET  /health                  status, deployed version, databases
//   GET  /version                 the deployed commit, from the VERSION
//                                 var its deploy stamps (#15793)
//
// <name> picks the D1 binding DB_<NAME> (dashes become underscores).
// Auth: "Authorization: Bearer <token>" whose SHA-256 hex is listed
// for that database in TOKEN_SHA256 (a JSON var: {"<name>": [hex...]}).
// Hashes only; token values never live in the repo.
//
// Other Workers call it over a service binding to the Gateway
// entrypoint: env.D1GW.pipeline("<name>", body), no token needed.
import { WorkerEntrypoint } from "cloudflare:workers";
import { handle, runPipeline } from "./http.js";

// Service-binding entrypoint for other Workers in the account.
export class Gateway extends WorkerEntrypoint {
  async pipeline(db, body) {
    return runPipeline(this.env, db, body);
  }
}

export default {
  fetch(request, env) {
    return handle(request, env);
  },
};
