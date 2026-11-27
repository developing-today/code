// This runs with only a GitHub read token, before the job attaches an
// environment (which could otherwise implicitly create a missing one).
import { releaseRepository, uploadEnvironment, promotionEnvironment, validateEnvironment } from "../src/deploy-contract.mjs";

const [environment, ...extra] = process.argv.slice(2);
if (extra.length || !(environment === uploadEnvironment || environment === promotionEnvironment)
  || process.env.GITHUB_REPOSITORY !== releaseRepository || process.env.GITHUB_REF !== "refs/heads/main"
  || process.env.GITHUB_EVENT_NAME !== "workflow_dispatch" || !process.env.GH_TOKEN) {
  throw new Error("Voice Line environment guard requires the exact manual main workflow");
}
const endpoint = `https://api.github.com/repos/${releaseRepository}/environments/${environment}`;
async function read(url) {
  const res = await fetch(url, {
    headers: { authorization: `Bearer ${process.env.GH_TOKEN}`, accept: "application/vnd.github+json" },
    redirect: "error",
    signal: AbortSignal.timeout(15_000),
  });
  if (!res.ok) throw new Error(`Voice Line environment protection is unavailable (HTTP ${res.status})`);
  return res.json();
}
const [structure, policy] = await Promise.all([read(endpoint), read(`${endpoint}/deployment-branch-policies?per_page=2`)]);
validateEnvironment(environment, structure, policy);
