import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {verifyBootstrapPostflight, verifyBootstrapPreflight, verifyDistinctWriteCredential, verifyEnvironmentProtection, verifyProviderState, ENVIRONMENT_NAME, WORKER_NAME, DO_CLASS} from './staging-bootstrap.mjs';

const accountId = 'a'.repeat(32);
const zoneId = '1'.repeat(32);
const workerId = '2'.repeat(32);
const namespaceId = '3'.repeat(32);
const sha = 'b'.repeat(40);
const versionId = '01234567-89ab-cdef-0123-456789abcdef';
const deploymentId = '11111111-2222-3333-4444-555555555555';
const readToken = 'synthetic-read-token-long-enough';
const writeToken = 'synthetic-write-token-long-enough';
const worker = {id:workerId,name:WORKER_NAME,logpush:false,observability:{enabled:false},references:{domains:[]}};
const version = () => ({
  id:versionId,
  annotations:{'workers/tag':sha},
  limits:{cpu_ms:10},
  exports:{[DO_CLASS]:{type:'durable-object',storage:'sqlite',state:'created'}},
  resources:{bindings:[
    {name:'ENABLED',type:'plain_text',text:'false'},
    {name:'GATEWAY_HOST',type:'plain_text',text:'private-bridge-staging.invalid'},
    {name:'GATEWAY_ACCOUNT_ID',type:'plain_text',text:''},
    {name:'GATEWAY_POLICY_ID',type:'plain_text',text:''},
    {name:'GATEWAY_IDENTITY_EMAIL',type:'plain_text',text:''},
    {name:'GATEWAY_POSTURE_ID',type:'plain_text',text:''},
    {name:'BRIDGE_REGISTRY',type:'plain_text',text:''},
    {name:'BRIDGE',type:'durable_object_namespace',class_name:DO_CLASS,namespace_id:namespaceId},
  ]},
});
const deployment = () => ({id:deploymentId,versions:[{version_id:versionId,percentage:100}]});
const ns = () => ({id:namespaceId,script:WORKER_NAME,class:DO_CLASS,use_sqlite:true});
const snapshot = () => ({
  zoneIds:[zoneId],worker,workerId,subdomain:{enabled:false,previews_enabled:false},
  settings:{logpush:false,observability:{enabled:false},tail_consumers:[]},schedules:[],
  namespaces:[ns()],domains:[],versions:[version()],deployments:[deployment()],
});

test('first-deploy workflow remains dormant, fixed-target, and checks protection before environment assignment', async()=>{
  const workflowPath=resolve('private-laptop-bridge-staging-bootstrap.workflow.yml.disabled');
  const workflow=await readFile(workflowPath,'utf8');
  assert.equal(workflowPath.includes('/.github/workflows/'),false);
  assert.match(workflow,/workflow_dispatch:/);
  assert.doesNotMatch(workflow,/^\s+inputs:/m);
  assert.match(workflow,/if: github\.repository == 'developing-today\/code' && github\.ref == 'refs\/heads\/main' && github\.event_name == 'workflow_dispatch'/);
  assert.match(workflow,/name: \$\{\{ needs\.verify-environment-before-assignment\.outputs\.environment \}\}/);
  assert.match(workflow,/needs: verify-environment-before-assignment/);
  assert.match(workflow,/CF_BRIDGE_BOOTSTRAP_REVIEWER_IDS: \$\{\{ vars\.CF_BRIDGE_BOOTSTRAP_REVIEWER_IDS \}\}/);
  const gate=workflow.split('  deploy-disabled-bridge:',1)[0];
  assert.doesNotMatch(gate,/secrets\./);
  const provider=workflow.split('      - name: Read current provider state')[1].split('      - name: Deploy only')[0];
  assert.match(provider,/CF_BRIDGE_BOOTSTRAP_READ_TOKEN/);
  assert.doesNotMatch(provider,/CF_BRIDGE_BOOTSTRAP_WRITE_TOKEN/);
  assert.match(workflow,/staging-bootstrap\.mjs assert-distinct-credentials/);
  assert.match(workflow,/cf deploy --mode bridge --prebuilt --provision --tag "\$GITHUB_SHA"/);
  assert.doesNotMatch(workflow,/wrangler deploy|workers versions (create|deploy)|--dispatch-namespace/);
  for(const fixed of [WORKER_NAME,'CF_BRIDGE_BOOTSTRAP_ACCOUNT_ID','CF_BRIDGE_BOOTSTRAP_ZONE_IDS',ENVIRONMENT_NAME]) assert.ok(workflow.includes(fixed));
});

function response(result, {status=200,resultInfo={page:1,per_page:100,total_count:Array.isArray(result)?result.length:0,total_pages:1},success=true,errors=[]}={}) {
  return new Response(JSON.stringify({success,result,errors,...(resultInfo === undefined ? {} : {result_info:resultInfo})}),{status});
}

function cloudflareFixture({target='absent', zones=[{id:zoneId,account:{id:accountId}}], zoneInfo, route=[], domains=[], namespaces=[], workerRecord=worker, versions=[version()], deployments=[deployment()], overrides={}}={}) {
  const calls=[];
  const fetchImpl=async (url,init={})=>{
    const parsed=new URL(url), method=init.method??'GET';
    calls.push({url:parsed,method,token:init.headers?.authorization});
    if(method!=='GET') return new Response('{}',{status:405});
    if(parsed.pathname==='/client/v4/zones') return response(zones,{resultInfo:zoneInfo??{page:1,per_page:50,total_count:zones.length,total_pages:zones.length?1:0}});
    if(parsed.pathname===`/client/v4/zones/${zoneId}/workers/routes`) return response(route,{resultInfo:undefined});
    if(parsed.pathname==='/client/v4/accounts/'+accountId+'/workers/domains') return response(domains);
    if(parsed.pathname==='/client/v4/accounts/'+accountId+'/workers/durable_objects/namespaces') return response(namespaces,{resultInfo:{page:1,per_page:1000,total_count:namespaces.length,total_pages:namespaces.length?1:0}});
    if(parsed.pathname===`/client/v4/accounts/${accountId}/workers/workers/${WORKER_NAME}`) {
      if(target==='absent') return response(null,{status:404,success:false,resultInfo:undefined,errors:[{code:10007}]});
      return response(workerRecord,{resultInfo:undefined});
    }
    if(parsed.pathname.endsWith(`/workers/scripts/${WORKER_NAME}/subdomain`)) return response({enabled:false,previews_enabled:false});
    if(parsed.pathname.endsWith(`/workers/scripts/${WORKER_NAME}/script-settings`)) return response({logpush:false,observability:{enabled:false},tail_consumers:[]});
    if(parsed.pathname.endsWith(`/workers/scripts/${WORKER_NAME}/schedules`)) return response({schedules:[]});
    if(parsed.pathname.endsWith(`/workers/workers/${workerId}/versions`)) return response(versions);
    if(parsed.pathname.endsWith(`/workers/scripts/${WORKER_NAME}/deployments`)) return response({deployments});
    const replacement=overrides[parsed.pathname];
    if(replacement) return replacement;
    throw new Error(`unexpected synthetic request ${method} ${parsed.pathname}`);
  };
  return {fetchImpl,calls};
}

test('GitHub dispatch gate requires the existing exact environment, exact independent reviewer set, no self-approval and main only',()=>{
  const environment={name:ENVIRONMENT_NAME,protection_rules:[{type:'required_reviewers',prevent_self_review:true,reviewers:[{type:'User',reviewer:{id:71}}]}],deployment_branch_policy:{protected_branches:false,custom_branch_policies:true}};
  const branchPolicies={total_count:1,branch_policies:[{name:'main'}]};
  assert.deepEqual(verifyEnvironmentProtection({environment,branchPolicies,expectedReviewerIds:'[71]'}),{environment:ENVIRONMENT_NAME});
  for(const change of [
    {environment:null},
    {environment:{...environment,name:'cloudflare-staging'}},
    {environment:{...environment,protection_rules:[{type:'required_reviewers',prevent_self_review:false,reviewers:[{type:'User',reviewer:{id:71}}]}]}},
    {environment:{...environment,protection_rules:[{type:'required_reviewers',prevent_self_review:true,reviewers:[{type:'User',reviewer:{id:72}}]}]}},
    {branchPolicies:{total_count:2,branch_policies:[{name:'main'},{name:'release/*'}]}},
    {branchPolicies:{total_count:1,branch_policies:[{name:'*'}]}},
  ]) assert.throws(()=>verifyEnvironmentProtection({environment:Object.hasOwn(change,'environment')?change.environment:environment,branchPolicies:change.branchPolicies??branchPolicies,expectedReviewerIds:'[71]'}));
  assert.throws(()=>verifyEnvironmentProtection({environment,branchPolicies,expectedReviewerIds:''}));
});

test('missing Worker is deployable only after complete read-only account inventory; all provider calls are GET',async()=>{
  const f=cloudflareFixture({target:'absent'});
  const status=await verifyBootstrapPreflight({accountId,readToken,expectedZoneIds:[zoneId],sourceSha:sha,fetchImpl:f.fetchImpl});
  assert.equal(status,'deploy');
  assert.ok(f.calls.every(call=>call.method==='GET'&&call.token===`Bearer ${readToken}`));
  assert.ok(f.calls.some(call=>call.url.pathname===`/client/v4/zones/${zoneId}/workers/routes`));
  for(const bad of [
    cloudflareFixture({target:'absent',zones:[]}),
    cloudflareFixture({target:'absent',route:[{id:'route',pattern:'*',script:WORKER_NAME}]}),
    cloudflareFixture({target:'absent',domains:[{service:WORKER_NAME,hostname:'unsafe.example'}]}),
    cloudflareFixture({target:'absent',namespaces:[ns()]}),
    cloudflareFixture({target:'absent',zoneInfo:{page:2,per_page:50,total_count:1,total_pages:1}}),
  ]) await assert.rejects(verifyBootstrapPreflight({accountId,readToken,expectedZoneIds:[zoneId],sourceSha:sha,fetchImpl:bad.fetchImpl}));
  assert.equal(verifyDistinctWriteCredential(readToken,writeToken),true);
  assert.throws(()=>verifyDistinctWriteCredential(readToken,readToken),/separate/);
});

test('existing Worker is a no-op only when disabled ingress, exact SQLite namespace, binding and serving version agree',()=>{
  const good={...snapshot(),domains:[]};
  assert.equal(verifyProviderState(good,{phase:'preflight',expectedZoneIds:[zoneId],sourceSha:sha}),'already-ready');
  assert.equal(verifyProviderState(good,{phase:'postflight',expectedZoneIds:[zoneId],sourceSha:sha}),'verified');
  assert.throws(()=>verifyProviderState(good,{phase:'preflight',expectedZoneIds:[zoneId],sourceSha:'c'.repeat(40)}),/exact reviewed source SHA/);
  for(const mutate of [
    s=>{s.zoneIds=[];},
    s=>{s.subdomain.enabled=true;},
    s=>{s.subdomain.previews_enabled=true;},
    s=>{s.schedules=[{cron:'* * * * *'}];},
    s=>{s.settings.logpush=true;},
    s=>{s.settings.tail_consumers=[{}];},
    s=>{s.domains=[{hostname:'unsafe.example'}];},
    s=>{s.namespaces[0].script='other-worker';},
    s=>{s.namespaces[0].use_sqlite=false;},
    s=>{s.deployments[0].versions[0].percentage=50;},
    s=>{s.versions[0].resources.bindings[0].text='true';},
    s=>{s.versions[0].resources.bindings.push({name:'OUTSIDE',type:'service'});},
    s=>{s.versions[0].exports[DO_CLASS].storage='legacy-kv';},
    s=>{s.versions[0].limits.cpu_ms=50;},
  ]) { const bad=snapshot(); mutate(bad); assert.throws(()=>verifyProviderState(bad,{phase:'preflight',expectedZoneIds:[zoneId],sourceSha:sha})); }
  assert.throws(()=>verifyProviderState(good,{phase:'postflight',expectedZoneIds:[zoneId],sourceSha:'c'.repeat(40)}),/exact reviewed source SHA/);
});

test('postflight readback verifies deployed identity via independent API data and never writes',async()=>{
  const f=cloudflareFixture({target:'existing',namespaces:[ns()]});
  assert.equal(await verifyBootstrapPostflight({accountId,readToken,expectedZoneIds:[zoneId],sourceSha:sha,fetchImpl:f.fetchImpl}),'verified');
  assert.ok(f.calls.every(call=>call.method==='GET'&&call.token===`Bearer ${readToken}`));
  const bad=cloudflareFixture({target:'existing',namespaces:[ns()],versions:[{...version(),annotations:{'workers/tag':'c'.repeat(40)}}]});
  await assert.rejects(verifyBootstrapPostflight({accountId,readToken,expectedZoneIds:[zoneId],sourceSha:sha,fetchImpl:bad.fetchImpl}),/exact reviewed source SHA/);
});
