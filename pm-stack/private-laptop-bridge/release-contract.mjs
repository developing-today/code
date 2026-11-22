import {createHash} from 'node:crypto';
import {lstat, readFile, readdir} from 'node:fs/promises';
import {join, relative, resolve, sep} from 'node:path';

const UUID=/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const SHA=/^[0-9a-f]{40}$/;
const DIGEST=/^[0-9a-f]{64}$/;
const NAMESPACE=/^(?:[0-9a-f]{32}|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$/;
const WORKFLOW='.github/workflows/private-laptop-bridge-release.yml';
export const releaseConfigFiles=['cloudflare.config.ts','wrangler.jsonc','wrangler.staging.jsonc','wrangler.smoke.jsonc','package.json','package-lock.json','staging-preflight.json'];
const fail=(message)=>{throw new Error(message);};
const digest=(bytes)=>createHash('sha256').update(bytes).digest('hex');
const stable=(value)=>Array.isArray(value)?value.map(stable):value&&typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(key=>[key,stable(value[key])])):value;
const same=(a,b)=>JSON.stringify(stable(a))===JSON.stringify(stable(b));

export function extractServerVersionId(stdout,{worker}={}) {
  let value; try {value=JSON.parse(stdout);} catch {fail('cf versions create did not return JSON; refusing to infer a version id');}
  if(value?.type!=='version-upload'||value?.version!==1||value?.worker_name!==worker) fail('cf versions create response does not match the pinned version-upload schema and exact Worker');
  const id=value?.version_id;
  if(typeof id!=='string'||!UUID.test(id)) fail('cf versions create response has no exact version UUID');
  return id;
}

export function provenanceDigest({worker,sourceSha,configDigest,buildDigest,preflightDigest}) {
  if(typeof worker!=='string'||!SHA.test(sourceSha)||!DIGEST.test(configDigest)||!DIGEST.test(buildDigest)||!DIGEST.test(preflightDigest)) fail('invalid provenance digest inputs');
  return digest(JSON.stringify({worker,sourceSha,configDigest,buildDigest,preflightDigest}));
}

export async function digestFiles(root, files=releaseConfigFiles) {
  const entries=[];
  for(const name of [...files].sort()) {
    if(name.startsWith('/')||name.split(/[\\/]/).includes('..')) fail('digest path escapes project');
    const path=join(root,name), stat=await lstat(path);
    if(!stat.isFile()||stat.isSymbolicLink()) fail(`digest input is not a regular file: ${name}`);
    entries.push([name,digest(await readFile(path))]);
  }
  return digest(JSON.stringify(entries));
}

export async function digestTree(root) {
  const abs=resolve(root), entries=[];
  async function walk(dir) {
    for(const item of (await readdir(dir,{withFileTypes:true})).sort((a,b)=>a.name.localeCompare(b.name))) {
      const path=join(dir,item.name), st=await lstat(path);
      if(st.isSymbolicLink()) fail(`Build Output contains symlink: ${relative(abs,path)}`);
      if(st.isDirectory()) await walk(path);
      else if(st.isFile()) entries.push([relative(abs,path).split(sep).join('/'),digest(await readFile(path))]);
      else fail(`Build Output contains unsupported entry: ${relative(abs,path)}`);
    }
  }
  await walk(abs); if(!entries.length) fail('Build Output is empty');
  return digest(JSON.stringify(entries));
}

function parseItems(text,key) {
  let body; try {body=JSON.parse(text);} catch {fail(`${key} response is not JSON`);}
  if(body?.success===false) fail(`${key} API returned success=false`);
  let rows=Array.isArray(body)?body:body?.result;
  if(!Array.isArray(rows)) rows=rows?.items ?? body?.[key];
  if(!Array.isArray(rows)) fail(`${key} response contains no result list`);
  return rows;
}

export function assertExactVersion({stdout,versionsJson,worker,sourceSha,configDigest,buildDigest,preflightDigest}) {
  if(!SHA.test(sourceSha)||!DIGEST.test(configDigest)||!DIGEST.test(buildDigest)||!DIGEST.test(preflightDigest)) fail('invalid source/config/build digest');
  const id=extractServerVersionId(stdout,{worker});
  const matches=parseItems(versionsJson,'versions').filter(v=>v.id===id);
  if(matches.length!==1) fail('server-returned version UUID is absent or ambiguous in the exact Worker version list');
  const version=matches[0], expectedTag=sourceSha, expectedProof=provenanceDigest({worker,sourceSha,configDigest,buildDigest,preflightDigest});
  const tag=version.annotations?.['workers/tag'];
  const message=version.annotations?.['workers/message'];
  if(tag!==expectedTag) fail('exact version metadata tag does not bind the full source SHA');
  if(typeof message!=='string'||!message.includes(`provenance=${expectedProof}`)) fail('exact version metadata message does not bind the full source/config/build/preflight provenance digest');
  return id;
}

const HANDOFF_KEYS=['schema','action','repository','workflow','upload_run_id','upload_run_attempt','source_sha','worker','environment','account_id','mode','version_id','tag','config_digest','build_digest','preflight_digest','provenance_digest','expected_disabled'];
export function makeHandoff(fields) {
  const handoff={schema:1,action:'upload',...fields};
  validateHandoffShape(handoff); return Object.fromEntries(HANDOFF_KEYS.map(k=>[k,handoff[k]]));
}
function validateHandoffShape(h) {
  if(!h||typeof h!=='object'||Array.isArray(h)||Object.keys(h).sort().join(',')!==[...HANDOFF_KEYS].sort().join(',')) fail('handoff fields are missing or unexpected');
  if(h.schema!==1||h.action!=='upload'||h.workflow!==WORKFLOW||!/^\d+$/.test(String(h.upload_run_id))||!Number.isInteger(h.upload_run_attempt)||h.upload_run_attempt<1||!SHA.test(h.source_sha)||!UUID.test(h.version_id)||!DIGEST.test(h.config_digest)||!DIGEST.test(h.build_digest)||!DIGEST.test(h.preflight_digest)||!DIGEST.test(h.provenance_digest)||h.provenance_digest!==provenanceDigest({worker:h.worker,sourceSha:h.source_sha,configDigest:h.config_digest,buildDigest:h.build_digest,preflightDigest:h.preflight_digest})||h.expected_disabled!==true) fail('handoff has invalid provenance or digest fields');
  if(h.repository!=='developing-today/code') fail('handoff repository mismatch');
  const target=h.worker==='private-laptop-bridge-staging'?'cloudflare-staging':h.worker==='private-laptop-bridge-smoke'?'cloudflare-staging-smoke':null;
  if(!target||h.environment!==target) fail('handoff Worker/environment mismatch');
  const mode=h.worker==='private-laptop-bridge-staging'?'bridge':'default';
  if(h.mode!==mode||h.tag!==h.source_sha||! /^[0-9a-f]{32}$/.test(h.account_id)) fail('handoff target/account/source mismatch');
}

export function verifyPromotionProvenance(handoff, expected) {
  validateHandoffShape(handoff);
  for(const key of ['repository','workflow','upload_run_id','upload_run_attempt','source_sha','worker','environment','account_id','mode','version_id','config_digest','build_digest','preflight_digest','provenance_digest']) {
    if(handoff[key]!==expected[key]) fail(`promotion provenance mismatch: ${key}`);
  }
  if(expected.action!=='upload'||expected.event!=='workflow_dispatch'||expected.branch!=='main'||expected.status!=='completed'||expected.conclusion!=='success'||expected.workflow_path!==WORKFLOW||expected.head_sha!==handoff.source_sha||expected.run_attempt!==handoff.upload_run_attempt||expected.repository!==handoff.repository) fail('upload run is not a successful trusted main-branch upload run');
  return true;
}

export function assertVersionStillMatches(versionsJson,handoff) {
  const rows=parseItems(versionsJson,'versions').filter(v=>v.id===handoff.version_id);
  if(rows.length!==1) fail('handoff version is not uniquely present on the exact Worker');
  const v=rows[0];
  if(v.annotations?.['workers/tag']!==handoff.tag) fail('Cloudflare version source tag differs from handoff');
  const m=v.annotations?.['workers/message']??'';
  if(!m.includes(`provenance=${handoff.provenance_digest}`)) fail('Cloudflare version digest metadata differs from handoff');
  return true;
}

export function assertDeployable(deployableJson,versionId) {
  if(!UUID.test(versionId)) fail('invalid target version UUID');
  if(!parseItems(deployableJson,'deployable versions').some(v=>v.id===versionId)) fail('exact handoff version is not deployable');
  return true;
}

export function assertDeploymentCreated(responseJson,versionId) {
  let body; try {body=JSON.parse(responseJson);} catch {fail('deployment-create response is not JSON');}
  if(body?.success===false) fail('deployment-create API returned success=false');
  const deployment=body?.result;
  if(!deployment||!UUID.test(deployment.id??'')||!Array.isArray(deployment.versions)||deployment.versions.length!==1||deployment.versions[0]?.version_id!==versionId||deployment.versions[0]?.percentage!==100) fail('created deployment identity or exact 100% version set is invalid');
  return deployment.id;
}

export function assertLatestServing(deploymentsJson,versionId,deploymentId) {
  if(!UUID.test(deploymentId??'')) fail('invalid deployment identity');
  const rows=parseItems(deploymentsJson,'deployments');
  const dated=rows.map(row=>({row,at:Date.parse(row.created_on??row.created_at??'')}));
  if(dated.some(x=>!Number.isFinite(x.at))) fail('deployment listing contains an invalid timestamp');
  const latestAt=Math.max(...dated.map(x=>x.at));
  const latest=dated.filter(x=>x.at===latestAt);
  if(latest.length!==1) fail('latest deployment is ambiguous');
  const traffic=latest[0].row.versions;
  if(latest[0].row.id!==deploymentId||!Array.isArray(traffic)||traffic.length!==1||traffic[0].version_id!==versionId||traffic[0].percentage!==100) fail('latest deployment is not the returned deployment identity with the exact complete 100% version set');
  return true;
}

export function assertUploadRun(run,jobsResponse,handoff) {
  if(!run||run.id!==Number(handoff.upload_run_id)||run.run_attempt!==handoff.upload_run_attempt||run.status!=='completed'||run.conclusion!=='success'||run.event!=='workflow_dispatch'||run.head_branch!=='main'||run.head_sha!==handoff.source_sha||run.path?.split('@')[0]!==WORKFLOW||run.repository?.full_name!==handoff.repository) fail('upload run is not the exact successful trusted main-branch workflow run');
  let jobs; try {jobs=typeof jobsResponse==='string'?JSON.parse(jobsResponse):jobsResponse;} catch {fail('upload workflow job details are not JSON');}
  const rows=jobs?.jobs;
  if(!Array.isArray(rows)||jobs.total_count!==1||rows.length!==1) fail('upload run must contain exactly one workflow job');
  const job=rows[0];
  if(job.name!=='release'||job.run_id!==run.id||job.head_sha!==handoff.source_sha||job.status!=='completed'||job.conclusion!=='success'||!Array.isArray(job.steps)) fail('upload workflow job is not the exact successful source job');
  const steps=new Map(job.steps.map(step=>[step.name,step]));
  for(const name of ['Upload an immutable candidate and write exact provenance artifact','Preserve candidate artifact for fourteen days']) {
    const step=steps.get(name); if(step?.status!=='completed'||step?.conclusion!=='success') fail(`trusted upload run did not complete the required artifact step: ${name}`);
  }
  for(const name of ['Verify artifact provenance, exact version and deployable set','Promote only the exact artifact version']) {
    const step=steps.get(name); if(step?.status!=='completed'||step?.conclusion!=='skipped') fail(`candidate run unexpectedly executed a promotion step: ${name}`);
  }
  return true;
}

export function assertPreflight(proof,{accountId,worker,environment,now=Date.now()}) {
  if(!proof||proof.schema!==1||proof.verified!==true||proof.account_id!==accountId||typeof proof.evidence_ref!=='string'||proof.evidence_ref.trim().length<3||!Number.isFinite(Date.parse(proof.verified_at))||!Number.isFinite(Date.parse(proof.expires_at))||Date.parse(proof.verified_at)>now||Date.parse(proof.expires_at)<=now||Date.parse(proof.expires_at)-Date.parse(proof.verified_at)>24*3600000) fail('owner read-only staging proof is absent, expired, or invalid');
  const release=proof.release_policy,allowedRefs=release?.allowed_environment_branches?.[environment];
  if(release?.repository!=='developing-today/code'||release?.branch!=='main'||release?.branch_protected!==true||release?.required_pull_request_reviews!==true||release?.force_push_disabled!==true||release?.admin_bypass_disabled!==true||release?.environment_reviewers_required?.[environment]!==true||release?.self_review_disabled?.[environment]!==true||!Array.isArray(allowedRefs)||!same(allowedRefs,['main'])||typeof release?.evidence_ref!=='string'||release.evidence_ref.trim().length<3) fail('trusted protected-main or deployment-environment approval proof is absent or invalid');
  const target=proof.targets?.[worker];
  const expectedVars=worker==='private-laptop-bridge-staging'?{ENABLED:'false',GATEWAY_HOST:'private-bridge-staging.invalid',GATEWAY_ACCOUNT_ID:'',GATEWAY_POLICY_ID:'',GATEWAY_IDENTITY_EMAIL:'',GATEWAY_POSTURE_ID:'',BRIDGE_REGISTRY:''}:{ENABLED:'false',GATEWAY_HOST:'private-bridge-smoke.invalid',GATEWAY_ACCOUNT_ID:'',GATEWAY_POLICY_ID:'',START_MS:'',END_MS:''};
  if(!target||target.environment!==environment||target.exists!==true||target.script_id!==worker||target.account_id!==accountId||target.workers_dev!==false||target.preview_urls!==false||!Array.isArray(target.routes)||target.routes.length!==0||!Array.isArray(target.custom_domains)||target.custom_domains.length!==0||target.application_enabled!==false||target.observability?.enabled!==false||target.observability?.logs!==false||target.observability?.traces!==false||target.limits?.cpu_ms!==10||!Array.isArray(target.gateway_forward_policy_ids)||target.gateway_forward_policy_ids.length!==0||!Array.isArray(target.gateway_caller_ids)||target.gateway_caller_ids.length!==0||JSON.stringify(Object.fromEntries(Object.entries(target.variables??{}).sort()))!==JSON.stringify(Object.fromEntries(Object.entries(expectedVars).sort()))||!Array.isArray(target.bindings)) fail('owner read-only proof does not verify the exact private disabled target');
  if(worker==='private-laptop-bridge-staging'&&(target.bindings.length!==1||target.bindings[0]?.name!=='BRIDGE'||target.bindings[0]?.type!=='durable_object_namespace'||target.bindings[0]?.class_name!=='PrivateLaptopBridge'||target.bindings[0]?.namespace_id!==target.do?.namespace_id||target.do?.worker_name!==worker||target.do?.class_name!=='PrivateLaptopBridge'||target.do?.storage!=='sqlite'||target.do?.export_state!=='created'||target.do?.migration_tag!=='v1'||target.do?.pending_migrations!==false||!NAMESPACE.test(target.do?.namespace_id??''))) fail('owner read-only proof does not verify the same-Worker SQLite Durable Object namespace and completed v1 migration state');
  if(worker==='private-laptop-bridge-smoke'&&(target.bindings.length!==0||target.do!==null)) fail('smoke target unexpectedly has a Durable Object namespace or binding');
  return true;
}

export function assertDryRunPayload(text,{worker,mode}) {
  if(typeof text!=='string'||!text.includes('--dry-run: exiting now.')) fail('cf prebuilt dry-run did not complete successfully');
  if(worker==='private-laptop-bridge-staging') {
    if(mode!=='bridge'||!text.includes('env.BRIDGE (PrivateLaptopBridge')||!text.includes('env.ENABLED ("false")')||!text.includes('env.BRIDGE_REGISTRY ("")')) fail('effective bridge Build Output does not match disabled DO target');
  } else if(worker==='private-laptop-bridge-smoke') {
    if(mode!=='default'||!text.includes('env.ENABLED ("false")')||!text.includes('env.GATEWAY_HOST ("private-bridge-smoke.invalid")')) fail('effective smoke Build Output does not match disabled target');
  } else fail('unknown Worker in Build Output verification');
  return true;
}

export function assertEffectivePayload(config,{worker,mode}) {
  const bridge=worker==='private-laptop-bridge-staging';
  const smoke=worker==='private-laptop-bridge-smoke';
  if((!bridge&&!smoke)||(bridge&&mode!=='bridge')||(smoke&&mode!=='default')) fail('Build Output mode does not match the fixed target');
  const common={
    name:worker,
    compatibilityDate:'2026-07-30',
    domains:[],
    triggers:[],
    limits:{cpuMs:10},
    observability:{enabled:false,logs:{enabled:false},traces:{enabled:false}},
    workersDev:false,
    previewUrls:false,
  };
  const variables=bridge?{
    ENABLED:'false',GATEWAY_HOST:'private-bridge-staging.invalid',GATEWAY_ACCOUNT_ID:'',GATEWAY_POLICY_ID:'',
    GATEWAY_IDENTITY_EMAIL:'',GATEWAY_POSTURE_ID:'',BRIDGE_REGISTRY:'',
  }:{
    ENABLED:'false',GATEWAY_HOST:'private-bridge-smoke.invalid',GATEWAY_ACCOUNT_ID:'',GATEWAY_POLICY_ID:'',START_MS:'',END_MS:'',
  };
  const expectedEnv=Object.fromEntries(Object.entries(variables).map(([name,value])=>[name,{type:'text',value}]));
  const allowedKeys=bridge
    ?['name','compatibilityDate','domains','triggers','limits','observability','workersDev','previewUrls','env','exports','manifest']
    :['name','compatibilityDate','domains','triggers','limits','observability','workersDev','previewUrls','env','manifest'];
  if(!config||typeof config!=='object'||Array.isArray(config)||Object.keys(config).sort().join(',')!==[...allowedKeys].sort().join(',')) fail('Build Output contains unexpected Worker settings or resources');
  for(const [key,value] of Object.entries(common)) if(!same(config[key],value)) fail(`effective Worker setting differs from the private target: ${key}`);
  if(!same(config.env,{...expectedEnv,...(bridge?{BRIDGE:{type:'durable-object',worker:'private-laptop-bridge-staging',exportName:'PrivateLaptopBridge'}}:{})})) fail('effective Worker variables or bindings differ from the fixed disabled target');
  if(config.manifest?.type!=='complete'||config.manifest?.mainModule!==(bridge?'gateway.js':'smoke.js')||!config.manifest.modules||typeof config.manifest.modules!=='object') fail('effective Worker module manifest is incomplete or targets the wrong entrypoint');
  const modules=Object.keys(config.manifest.modules).sort();
  const expectedModules=bridge?['gateway.js','gateway.js.map']:['smoke.js','smoke.js.map'];
  if(!same(modules,expectedModules)) fail('effective Worker module set differs from the reviewed entrypoint');
  if(bridge&&!same(config.exports,{PrivateLaptopBridge:{type:'durable-object',storage:'sqlite'}})) fail('effective bridge DO class or SQLite storage differs from the reviewed target');
  return true;
}
