import test from 'node:test';
import assert from 'node:assert/strict';
import {assertDeployable,assertDryRunPayload,assertEffectivePayload,assertExactVersion,assertLatestServing,assertPreflight,assertUploadRun,assertVersionStillMatches,makeHandoff,provenanceDigest,verifyPromotionProvenance} from './release-contract.mjs';

const uuid='01234567-89ab-cdef-0123-456789abcdef', old='11111111-2222-3333-4444-555555555555';
const sha='a'.repeat(40), cd='b'.repeat(64), bd='c'.repeat(64), pd='e'.repeat(64), acct='d'.repeat(32);
const proof=provenanceDigest({worker:'private-laptop-bridge-staging',sourceSha:sha,configDigest:cd,buildDigest:bd,preflightDigest:pd});
const version={id:uuid,annotations:{'workers/tag':sha,'workers/message':`release provenance=${proof}`}};
const list=rows=>JSON.stringify({success:true,result:{items:rows}});
const cfVersionResponse=(fields={})=>JSON.stringify({type:'version-upload',version:1,worker_name:'private-laptop-bridge-staging',worker_tag:'worker-identity-guid',version_id:uuid,timestamp:'2026-11-22T12:00:00.000Z',...fields});
const env={CF_ACCOUNT_ID:acct};
const handoff=()=>makeHandoff({repository:'developing-today/code',workflow:'.github/workflows/private-laptop-bridge-release.yml',upload_run_id:'501',upload_run_attempt:1,source_sha:sha,worker:'private-laptop-bridge-staging',environment:'cloudflare-staging',account_id:acct,mode:'bridge',version_id:uuid,tag:sha,config_digest:cd,build_digest:bd,preflight_digest:pd,provenance_digest:proof,expected_disabled:true});

test('captures the exact UUID returned by cf and refuses tag-list aliasing',()=>{
 assert.equal(assertExactVersion({stdout:cfVersionResponse(),versionsJson:list([version,{...version,id:old}]),worker:'private-laptop-bridge-staging',sourceSha:sha,configDigest:cd,buildDigest:bd,preflightDigest:pd}),uuid);
 assert.throws(()=>assertExactVersion({stdout:'uploaded successfully',versionsJson:list([version]),worker:'private-laptop-bridge-staging',sourceSha:sha,configDigest:cd,buildDigest:bd,preflightDigest:pd}),/did not return JSON/);
 assert.throws(()=>assertExactVersion({stdout:cfVersionResponse(),versionsJson:list([{...version,id:old}]),worker:'private-laptop-bridge-staging',sourceSha:sha,configDigest:cd,buildDigest:bd,preflightDigest:pd}),/absent or ambiguous/);
 assert.throws(()=>assertExactVersion({stdout:cfVersionResponse({version_id:'not-a-uuid'}),versionsJson:list([version]),worker:'private-laptop-bridge-staging',sourceSha:sha,configDigest:cd,buildDigest:bd,preflightDigest:pd}),/no exact version UUID/);
 assert.throws(()=>assertExactVersion({stdout:cfVersionResponse({version_id:null}),versionsJson:list([version]),worker:'private-laptop-bridge-staging',sourceSha:sha,configDigest:cd,buildDigest:bd,preflightDigest:pd}),/no exact version UUID/);
 assert.throws(()=>assertExactVersion({stdout:cfVersionResponse({worker_name:'private-laptop-bridge-smoke'}),versionsJson:list([version]),worker:'private-laptop-bridge-staging',sourceSha:sha,configDigest:cd,buildDigest:bd,preflightDigest:pd}),/does not match the pinned version-upload schema/);
 assert.throws(()=>assertExactVersion({stdout:cfVersionResponse({version:2}),versionsJson:list([version]),worker:'private-laptop-bridge-staging',sourceSha:sha,configDigest:cd,buildDigest:bd,preflightDigest:pd}),/does not match the pinned version-upload schema/);
 assert.throws(()=>assertExactVersion({stdout:JSON.stringify({success:true,result:version}),versionsJson:list([version]),worker:'private-laptop-bridge-staging',sourceSha:sha,configDigest:cd,buildDigest:bd,preflightDigest:pd}),/does not match the pinned version-upload schema/);
});

test('exact version must bind full SHA, exact config and actual Build Output digest',()=>{
 for(const v of [{...version,annotations:{...version.annotations,'workers/tag':sha.slice(0,12)}},{...version,annotations:{...version.annotations,'workers/message':`release provenance=${'e'.repeat(64)}`}},{...version,annotations:{...version.annotations,'workers/message':`release provenance=${'e'.repeat(64)}`}}]) assert.throws(()=>assertExactVersion({stdout:cfVersionResponse(),versionsJson:list([v]),worker:'private-laptop-bridge-staging',sourceSha:sha,configDigest:cd,buildDigest:bd,preflightDigest:pd}));
});

test('promotion handoff binds exact source, Worker, environment, account, upload run and attempt',()=>{
 const h=handoff();
 const base={repository:'developing-today/code',workflow:'.github/workflows/private-laptop-bridge-release.yml',upload_run_id:'501',upload_run_attempt:1,source_sha:sha,worker:'private-laptop-bridge-staging',environment:'cloudflare-staging',account_id:acct,mode:'bridge',version_id:uuid,config_digest:cd,build_digest:bd,preflight_digest:pd,provenance_digest:proof,action:'upload',event:'workflow_dispatch',branch:'main',status:'completed',conclusion:'success',workflow_path:'.github/workflows/private-laptop-bridge-release.yml',head_sha:sha,run_attempt:1};
 assert.equal(verifyPromotionProvenance(h,base),true);
 for(const [key,value] of [['upload_run_id','502'],['worker','private-laptop-bridge-smoke'],['environment','cloudflare-staging-smoke'],['account_id','e'.repeat(32)],['config_digest','f'.repeat(64)],['build_digest','f'.repeat(64)],['preflight_digest','f'.repeat(64)],['provenance_digest','f'.repeat(64)],['upload_run_attempt',2],['event','pull_request'],['branch','feature'],['status','in_progress'],['conclusion','failure'],['head_sha','f'.repeat(40)],['workflow_path','other.yml']]) assert.throws(()=>verifyPromotionProvenance(h,{...base,[key]:value}),undefined,key);
 const bad={...h,expected_disabled:false}; assert.throws(()=>verifyPromotionProvenance(bad,base),/invalid provenance/);
});

test('exact version is revalidated against Cloudflare metadata and deployable IDs',()=>{
 const h=handoff(); assert.equal(assertVersionStillMatches(list([version]),h),true); assert.equal(assertDeployable(list([{id:uuid}]),uuid),true);
 assert.throws(()=>assertVersionStillMatches(list([{...version,annotations:{...version.annotations,'workers/tag':'git-short'}}]),h));
 assert.throws(()=>assertDeployable(list([{id:old}]),uuid));
});

test('trusted workflow run must be exact successful main upload attempt',()=>{
 const h=handoff();
 const run={id:501,run_attempt:1,status:'completed',conclusion:'success',event:'workflow_dispatch',head_branch:'main',head_sha:sha,path:'.github/workflows/private-laptop-bridge-release.yml@refs/heads/main',repository:{full_name:'developing-today/code'}};
 const jobs={total_count:1,jobs:[{run_id:501,head_sha:sha,name:'release',status:'completed',conclusion:'success',steps:[
   {name:'Upload an immutable candidate and write exact provenance artifact',status:'completed',conclusion:'success'},
   {name:'Preserve candidate artifact for fourteen days',status:'completed',conclusion:'success'},
   {name:'Verify artifact provenance, exact version and deployable set',status:'completed',conclusion:'skipped'},
   {name:'Promote only the exact artifact version',status:'completed',conclusion:'skipped'},
 ]}]};
 assert.equal(assertUploadRun(run,jobs,h),true);
 for(const change of [{id:502},{run_attempt:2},{status:'in_progress'},{conclusion:'failure'},{event:'pull_request'},{head_branch:'feature'},{head_sha:'f'.repeat(40)},{repository:{full_name:'fork/evil'}}]) assert.throws(()=>assertUploadRun({...run,...change},h));
 assert.throws(()=>assertUploadRun(run,{...jobs,total_count:2},h),/exactly one workflow job/);
 for(const change of [
   {name:'Upload an immutable candidate and write exact provenance artifact',conclusion:'failure'},
   {name:'Preserve candidate artifact for fourteen days',conclusion:'skipped'},
   {name:'Promote only the exact artifact version',conclusion:'success'},
 ]) assert.throws(()=>assertUploadRun(run,{...jobs,jobs:[{...jobs.jobs[0],steps:jobs.jobs[0].steps.map(step=>step.name===change.name?{...step,conclusion:change.conclusion}:step)}]},h));
});

test('stale historical deployment cannot mask a newer wrong or split deployment',()=>{
 const deployments=rows=>JSON.stringify({success:true,result:rows});
 const serving={id:'new',created_on:'2026-11-22T12:00:00Z',versions:[{version_id:uuid,percentage:100}]};
 const oldServing={id:'old',created_on:'2026-11-22T11:00:00Z',versions:[{version_id:uuid,percentage:100}]};
 assert.equal(assertLatestServing(deployments([oldServing,serving]),uuid),true);
 assert.throws(()=>assertLatestServing(deployments([oldServing,{...serving,versions:[{version_id:old,percentage:100}]}]),uuid),/latest deployment/);
 assert.throws(()=>assertLatestServing(deployments([oldServing,{...serving,versions:[{version_id:uuid,percentage:50},{version_id:old,percentage:50}]}]),uuid),/latest deployment/);
 assert.throws(()=>assertLatestServing(deployments([oldServing,{...serving,versions:[{version_id:uuid,percentage:100},{version_id:old,percentage:0}]}]),uuid),/latest deployment/);
});

test('read-only owner proof fails closed for missing, stale, public or unexpected callers',()=>{
 const now=Date.parse('2026-11-22T12:00:00Z');
 const good={schema:1,verified:true,account_id:acct,reviewed_by:'owner@example.com',verified_at:'2026-11-22T11:00:00Z',expires_at:'2026-11-22T13:00:00Z',evidence_ref:'review-1',release_policy:{repository:'developing-today/code',branch:'main',branch_protected:true,required_pull_request_reviews:true,force_push_disabled:true,admin_bypass_disabled:true,environment_reviewers_required:{'cloudflare-staging':true,'cloudflare-staging-smoke':true},self_review_disabled:{'cloudflare-staging':true,'cloudflare-staging-smoke':true},allowed_environment_branches:{'cloudflare-staging':['main'],'cloudflare-staging-smoke':['main']},evidence_ref:'policy-review-1'},targets:{'private-laptop-bridge-staging':{environment:'cloudflare-staging',exists:true,script_id:'private-laptop-bridge-staging',account_id:acct,application_enabled:false,workers_dev:false,preview_urls:false,routes:[],custom_domains:[],observability:{enabled:false,logs:false,traces:false},limits:{cpu_ms:10},gateway_forward_policy_ids:[],gateway_caller_ids:[],variables:{ENABLED:'false',GATEWAY_HOST:'private-bridge-staging.invalid',GATEWAY_ACCOUNT_ID:'',GATEWAY_POLICY_ID:'',GATEWAY_IDENTITY_EMAIL:'',GATEWAY_POSTURE_ID:'',BRIDGE_REGISTRY:''},bindings:[{name:'BRIDGE',type:'durable_object_namespace',class_name:'PrivateLaptopBridge',namespace_id:'1'.repeat(32)}],do:{worker_name:'private-laptop-bridge-staging',class_name:'PrivateLaptopBridge',storage:'sqlite',export_state:'created',migration_tag:'v1',pending_migrations:false,namespace_id:'1'.repeat(32)}}}};
 assert.equal(assertPreflight(good,{accountId:acct,worker:'private-laptop-bridge-staging',environment:'cloudflare-staging',now}),true);
 assert.throws(()=>assertPreflight({...good,verified:false},{accountId:acct,worker:'private-laptop-bridge-staging',environment:'cloudflare-staging',now}),/absent, expired, or invalid/);
 assert.throws(()=>assertPreflight({...good,expires_at:'2026-11-22T11:59:59Z'},{accountId:acct,worker:'private-laptop-bridge-staging',environment:'cloudflare-staging',now}),/absent, expired, or invalid/);
 assert.throws(()=>assertPreflight({...good,evidence_ref:''},{accountId:acct,worker:'private-laptop-bridge-staging',environment:'cloudflare-staging',now}),/absent, expired, or invalid/);
 assert.throws(()=>assertPreflight({...good,release_policy:{...good.release_policy,branch_protected:false}},{accountId:acct,worker:'private-laptop-bridge-staging',environment:'cloudflare-staging',now}),/protected-main/);
 assert.throws(()=>assertPreflight({...good,release_policy:{...good.release_policy,environment_reviewers_required:{'cloudflare-staging':false}}},{accountId:acct,worker:'private-laptop-bridge-staging',environment:'cloudflare-staging',now}),/protected-main/);
 for(const change of [{gateway_caller_ids:['unexpected-caller']},{routes:['https://public.example/*']},{custom_domains:['public.example']},{workers_dev:true},{preview_urls:true},{gateway_forward_policy_ids:['unexpected-policy']},{observability:{enabled:false,logs:true,traces:false}},{limits:{cpu_ms:50}},{application_enabled:true},{variables:{...good.targets['private-laptop-bridge-staging'].variables,ENABLED:'true'}},{bindings:[]},{do:{...good.targets['private-laptop-bridge-staging'].do,worker_name:'other-worker'}},{do:{...good.targets['private-laptop-bridge-staging'].do,export_state:'pending'}},{do:{...good.targets['private-laptop-bridge-staging'].do,migration_tag:'v0'}},{do:{...good.targets['private-laptop-bridge-staging'].do,pending_migrations:true}}]) {
   const target={...good.targets['private-laptop-bridge-staging'],...change};
   assert.throws(()=>assertPreflight({...good,targets:{...good.targets,'private-laptop-bridge-staging':target}},{accountId:acct,worker:'private-laptop-bridge-staging',environment:'cloudflare-staging',now}));
 }
 const smokeTarget={environment:'cloudflare-staging-smoke',exists:true,script_id:'private-laptop-bridge-smoke',account_id:acct,application_enabled:false,workers_dev:false,preview_urls:false,routes:[],custom_domains:[],observability:{enabled:false,logs:false,traces:false},limits:{cpu_ms:10},gateway_forward_policy_ids:[],gateway_caller_ids:[],variables:{ENABLED:'false',GATEWAY_HOST:'private-bridge-smoke.invalid',GATEWAY_ACCOUNT_ID:'',GATEWAY_POLICY_ID:'',START_MS:'',END_MS:''},bindings:[],do:null};
 assert.equal(assertPreflight({...good,targets:{'private-laptop-bridge-smoke':smokeTarget}},{accountId:acct,worker:'private-laptop-bridge-smoke',environment:'cloudflare-staging-smoke',now}),true);
});

test('Versions release proof separately requires an existing Worker and its ready same-Worker DO namespace',()=>{
 const now=Date.parse('2026-11-22T12:00:00Z');
 const policy={repository:'developing-today/code',branch:'main',branch_protected:true,required_pull_request_reviews:true,force_push_disabled:true,admin_bypass_disabled:true,environment_reviewers_required:{'cloudflare-staging':true},self_review_disabled:{'cloudflare-staging':true},allowed_environment_branches:{'cloudflare-staging':['main']},evidence_ref:'policy-review-1'};
 const bridge={environment:'cloudflare-staging',exists:true,script_id:'private-laptop-bridge-staging',account_id:acct,application_enabled:false,workers_dev:false,preview_urls:false,routes:[],custom_domains:[],observability:{enabled:false,logs:false,traces:false},limits:{cpu_ms:10},gateway_forward_policy_ids:[],gateway_caller_ids:[],variables:{ENABLED:'false',GATEWAY_HOST:'private-bridge-staging.invalid',GATEWAY_ACCOUNT_ID:'',GATEWAY_POLICY_ID:'',GATEWAY_IDENTITY_EMAIL:'',GATEWAY_POSTURE_ID:'',BRIDGE_REGISTRY:''},bindings:[{name:'BRIDGE',type:'durable_object_namespace',class_name:'PrivateLaptopBridge',namespace_id:'1'.repeat(32)}],do:{worker_name:'private-laptop-bridge-staging',class_name:'PrivateLaptopBridge',storage:'sqlite',export_state:'created',migration_tag:'v1',pending_migrations:false,namespace_id:'1'.repeat(32)}};
 const makeProof=(target=bridge)=>({schema:1,verified:true,account_id:acct,reviewed_by:'owner@example.com',verified_at:'2026-11-22T11:00:00Z',expires_at:'2026-11-22T13:00:00Z',evidence_ref:'worker-review-1',release_policy:policy,targets:{'private-laptop-bridge-staging':target}});
 const check=(proof)=>assertPreflight(proof,{accountId:acct,worker:'private-laptop-bridge-staging',environment:'cloudflare-staging',now});
 assert.equal(check(makeProof()),true);
 assert.throws(()=>check(makeProof({...bridge,exists:false})),/exact private disabled target/);
 for(const change of [
   {do:{...bridge.do,worker_name:'private-laptop-bridge-smoke'}},
   {do:{...bridge.do,export_state:'missing'}},
   {do:{...bridge.do,migration_tag:'v0'}},
   {do:{...bridge.do,pending_migrations:true}},
   {do:{...bridge.do,namespace_id:''}},
   {bindings:[{...bridge.bindings[0],namespace_id:'2'.repeat(32)}]},
   {bindings:[{...bridge.bindings[0],class_name:'OtherClass'}]},
 ]) assert.throws(()=>check(makeProof({...bridge,...change})),/same-Worker SQLite Durable Object namespace and completed v1 migration state/);
});

test('CI dry-run verifies the effective private Build Output but does not claim account route proof',()=>{
 const bridge='env.BRIDGE (PrivateLaptopBridge, defined in private-laptop-bridge-staging)\nenv.ENABLED ("false")\nenv.BRIDGE_REGISTRY ("")\n--dry-run: exiting now.';
 assert.equal(assertDryRunPayload(bridge,{worker:'private-laptop-bridge-staging',mode:'bridge'}),true);
 assert.throws(()=>assertDryRunPayload(bridge.replace('env.ENABLED ("false")','env.ENABLED ("true")'),{worker:'private-laptop-bridge-staging',mode:'bridge'}));
 assert.throws(()=>assertDryRunPayload(bridge.replace('env.BRIDGE (','env.OTHER ('),{worker:'private-laptop-bridge-staging',mode:'bridge'}));
 assert.throws(()=>assertDryRunPayload('no output',{worker:'private-laptop-bridge-staging',mode:'bridge'}));
});

test('effective Build Output explicitly fixes CPU, observability, routes, preview, flags, variables and resource bindings',()=>{
 const base={name:'private-laptop-bridge-staging',compatibilityDate:'2026-07-30',domains:[],triggers:[],limits:{cpuMs:10},observability:{enabled:false,logs:{enabled:false},traces:{enabled:false}},workersDev:false,previewUrls:false,env:{ENABLED:{type:'text',value:'false'},GATEWAY_HOST:{type:'text',value:'private-bridge-staging.invalid'},GATEWAY_ACCOUNT_ID:{type:'text',value:''},GATEWAY_POLICY_ID:{type:'text',value:''},GATEWAY_IDENTITY_EMAIL:{type:'text',value:''},GATEWAY_POSTURE_ID:{type:'text',value:''},BRIDGE_REGISTRY:{type:'text',value:''},BRIDGE:{type:'durable-object',worker:'private-laptop-bridge-staging',exportName:'PrivateLaptopBridge'}},exports:{PrivateLaptopBridge:{type:'durable-object',storage:'sqlite'}},manifest:{type:'complete',mainModule:'gateway.js',modules:{'gateway.js':{type:'esm'},'gateway.js.map':{type:'sourcemap'}}}};
 assert.equal(assertEffectivePayload(base,{worker:'private-laptop-bridge-staging',mode:'bridge'}),true);
 for(const mutation of [x=>x.limits.cpuMs=30,x=>x.observability.enabled=true,x=>x.workersDev=true,x=>x.previewUrls=true,x=>x.domains.push('public.example'),x=>x.triggers.push({route:'public.example/*'}),x=>x.env.ENABLED.value='true',x=>x.env.EXTRA={type:'text',value:'secret'},x=>x.env.BRIDGE.worker='other-worker',x=>x.exports.PrivateLaptopBridge.storage='kv',x=>x.forwardPolicies=['unexpected']]) {
   const mutated=structuredClone(base); mutation(mutated); assert.throws(()=>assertEffectivePayload(mutated,{worker:'private-laptop-bridge-staging',mode:'bridge'}));
 }
 const smoke={name:'private-laptop-bridge-smoke',compatibilityDate:'2026-07-30',domains:[],triggers:[],limits:{cpuMs:10},observability:{enabled:false,logs:{enabled:false},traces:{enabled:false}},workersDev:false,previewUrls:false,env:{ENABLED:{type:'text',value:'false'},GATEWAY_HOST:{type:'text',value:'private-bridge-smoke.invalid'},GATEWAY_ACCOUNT_ID:{type:'text',value:''},GATEWAY_POLICY_ID:{type:'text',value:''},START_MS:{type:'text',value:''},END_MS:{type:'text',value:''}},manifest:{type:'complete',mainModule:'smoke.js',modules:{'smoke.js':{type:'esm'},'smoke.js.map':{type:'sourcemap'}}}};
 assert.equal(assertEffectivePayload(smoke,{worker:'private-laptop-bridge-smoke',mode:'default'}),true);
 for(const mutated of [{...smoke,exports:{}},{...smoke,env:{...smoke.env,BRIDGE:{type:'durable-object'}}},{...smoke,observability:{enabled:true}}]) assert.throws(()=>assertEffectivePayload(mutated,{worker:'private-laptop-bridge-smoke',mode:'default'}));
});
