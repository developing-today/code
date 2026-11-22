#!/usr/bin/env node
import {readFile,writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {assertDeployable,assertDryRunPayload,assertEffectivePayload,assertExactVersion,assertLatestServing,assertPreflight,assertUploadRun,assertVersionStillMatches,digestFiles,digestTree,makeHandoff,provenanceDigest} from './release-contract.mjs';
const [action,...args]=process.argv.slice(2);
const required=(name)=>{const value=process.env[name];if(!value)throw new Error(`missing ${name}`);return value;};
const json=async path=>JSON.parse(await readFile(path,'utf8'));
const write=(path,value)=>writeFile(path,JSON.stringify(value,null,2)+'\n',{mode:0o600});
const root=process.cwd();

if(action==='preflight') {
 const worker=required('TARGET_WORKER'), environment=required('TARGET_ENV'), accountId=required('CF_ACCOUNT_ID');
 const proof=await json(resolve(root,'staging-preflight.json'));
 assertPreflight(proof,{accountId,worker,environment});
 console.log('owner read-only staging proof valid for '+worker);
} else if(action==='digest') {
 const config_digest=await digestFiles(root), preflight_digest=await digestFiles(root,['staging-preflight.json']), build_digest=await digestTree(resolve(root,'.cloudflare/output/v1'));
 console.log(JSON.stringify({config_digest,preflight_digest,build_digest}));
} else if(action==='dry-run') {
 const worker=required('TARGET_WORKER'), mode=required('TARGET_MODE');
 assertDryRunPayload(await readFile(required('DRY_RUN_OUTPUT'),'utf8'),{worker,mode});
 const metadata=await json(resolve(root,'.cloudflare/output/v1/config.json'));
 if(metadata.producer?.name!=='wrangler'||metadata.producer?.version!=='4.166.0'||metadata.buildContext?.mode!==(mode==='bridge'?'bridge':undefined)) throw new Error('Build Output producer or mode does not match pinned release');
 const payload=await json(resolve(root,'.cloudflare/output/v1/workers/default/worker.config.json'));
 assertEffectivePayload(payload,{worker,mode});
 console.log('effective Build Output validated for '+worker);
} else if(action==='make-handoff') {
 const [responsePath,versionsPath,outputPath]=args;
 if(!responsePath||!versionsPath||!outputPath) throw new Error('usage: make-handoff <version-response.json> <versions.json> <output.json>');
 const source_sha=required('GITHUB_SHA'), worker=required('TARGET_WORKER'), environment=required('TARGET_ENV'), account_id=required('CF_ACCOUNT_ID'), mode=required('TARGET_MODE');
 const config_digest=await digestFiles(root), preflight_digest=await digestFiles(root,['staging-preflight.json']), build_digest=await digestTree(resolve(root,'.cloudflare/output/v1'));
 const stdout=await readFile(responsePath,'utf8'), versionsJson=await readFile(versionsPath,'utf8'), tag=source_sha;
 const version_id=assertExactVersion({stdout,versionsJson,worker,sourceSha:source_sha,configDigest:config_digest,buildDigest:build_digest,preflightDigest:preflight_digest});
 const artifact=makeHandoff({repository:required('GITHUB_REPOSITORY'),workflow:'.github/workflows/private-laptop-bridge-release.yml',upload_run_id:required('GITHUB_RUN_ID'),upload_run_attempt:Number(required('GITHUB_RUN_ATTEMPT')),source_sha,worker,environment,account_id,mode,version_id,tag,config_digest,build_digest,preflight_digest,provenance_digest:provenanceDigest({worker,sourceSha:source_sha,configDigest:config_digest,buildDigest:build_digest,preflightDigest:preflight_digest}),expected_disabled:true});
 await write(outputPath,artifact); console.log(version_id);
} else if(action==='provenance') {
 const [handoffPath,runPath]=args;
 const [,,jobsPath]=args;
 if(!handoffPath||!runPath||!jobsPath) throw new Error('usage: provenance <handoff.json> <run.json> <jobs.json>');
 const artifact=await json(handoffPath),run=await json(runPath),jobs=await json(jobsPath),worker=required('TARGET_WORKER'),environment=required('TARGET_ENV'),account=required('CF_ACCOUNT_ID'),mode=required('TARGET_MODE');
 const expectedMode=worker==='private-laptop-bridge-staging'?'bridge':worker==='private-laptop-bridge-smoke'?'default':null;
 if(!expectedMode||artifact.worker!==worker||artifact.environment!==environment||artifact.account_id!==account||artifact.mode!==expectedMode||mode!==expectedMode) throw new Error('upload artifact target/environment/account/mode mismatch');
 assertUploadRun(run,jobs,artifact); console.log(artifact.source_sha);
} else if(action==='current-proof') {
 const [handoffPath]=args; if(!handoffPath) throw new Error('usage: current-proof <handoff.json>');
 const artifact=await json(handoffPath),account=required('CF_ACCOUNT_ID'),worker=required('TARGET_WORKER'),environment=required('TARGET_ENV');
 if(artifact.account_id!==account||artifact.worker!==worker||artifact.environment!==environment) throw new Error('current owner proof target differs from upload artifact');
 const proof=await json(resolve(root,'staging-preflight.json')); assertPreflight(proof,{accountId:account,worker,environment});
 const preflight_digest=await digestFiles(root,['staging-preflight.json']);
 if(preflight_digest!==artifact.preflight_digest) throw new Error('current main owner proof changed since upload; upload a new candidate');
 console.log('current main owner proof matches uploaded candidate');
} else if(action==='verify-handoff') {
 const [handoffPath,runPath,jobsPath,versionsPath,deployablePath]=args;
 if(!handoffPath||!runPath||!jobsPath||!versionsPath||!deployablePath) throw new Error('usage: verify-handoff <handoff.json> <run.json> <jobs.json> <versions.json> <deployable.json>');
 const artifact=await json(handoffPath), run=await json(runPath),jobs=await json(jobsPath), target=required('TARGET_WORKER'), account=required('CF_ACCOUNT_ID'), env=required('TARGET_ENV'), mode=required('TARGET_MODE');
 const expectedMode=target==='private-laptop-bridge-staging'?'bridge':'default';
 if(artifact.worker!==target||artifact.account_id!==account||artifact.environment!==env||artifact.mode!==expectedMode||mode!==expectedMode) throw new Error('promotion target/environment/account/mode differs from handoff');
 assertUploadRun(run,jobs,artifact);
 const proof=await json(resolve(root,'staging-preflight.json')); assertPreflight(proof,{accountId:account,worker:target,environment:env});
 const config_digest=await digestFiles(root),preflight_digest=await digestFiles(root,['staging-preflight.json']),build_digest=await digestTree(resolve(root,'.cloudflare/output/v1'));
 const head=required('CHECKED_OUT_SHA');
 if(head!==artifact.source_sha||config_digest!==artifact.config_digest||preflight_digest!==artifact.preflight_digest||build_digest!==artifact.build_digest) throw new Error('checked-out source/config/build/preflight digest does not equal uploaded artifact');
 assertVersionStillMatches(await readFile(versionsPath,'utf8'),artifact);
 assertDeployable(await readFile(deployablePath,'utf8'),artifact.version_id);
 console.log(`verified ${artifact.version_id} from run ${artifact.upload_run_id} at ${artifact.source_sha}`);
} else if(action==='verify-deployment') {
 const [deploymentPath,handoffPath]=args;
 if(!deploymentPath||!handoffPath) throw new Error('usage: verify-deployment <deployments.json> <handoff.json>');
 const artifact=await json(handoffPath);assertLatestServing(await readFile(deploymentPath,'utf8'),artifact.version_id);
 console.log(`latest deployment is exactly ${artifact.version_id} at 100%`);
} else throw new Error('action must be preflight, digest, dry-run, make-handoff, provenance, verify-handoff, or verify-deployment');
