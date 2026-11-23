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
const domainId = '22222222-3333-4444-5555-666666666666';
const readToken = 'synthetic-read-token-long-enough';
const writeToken = 'synthetic-write-token-long-enough';
const gatewayToken = 'synthetic-gateway-read-token-long-enough';
const worker = {id:workerId,name:WORKER_NAME,logpush:false,observability:{enabled:false},references:{domains:[],workers:[],durable_objects:[],dispatch_namespace_outbounds:[],queues:[]}};
const bindings = () => [
    {name:'ENABLED',type:'plain_text',text:'false'},
    {name:'GATEWAY_HOST',type:'plain_text',text:'private-bridge-staging.invalid'},
    {name:'GATEWAY_ACCOUNT_ID',type:'plain_text',text:''},
    {name:'GATEWAY_POLICY_ID',type:'plain_text',text:''},
    {name:'GATEWAY_IDENTITY_EMAIL',type:'plain_text',text:''},
    {name:'GATEWAY_POSTURE_ID',type:'plain_text',text:''},
    {name:'BRIDGE_HOST',type:'plain_text',text:'laptop-bridge-staging.security.cab'},
    {name:'ACCESS_ISSUER',type:'plain_text',text:''},
    {name:'ACCESS_AUDIENCE',type:'plain_text',text:''},
    {name:'BRIDGE_REGISTRY',type:'plain_text',text:''},
    {name:'BRIDGE',type:'durable_object_namespace',class_name:DO_CLASS,namespace_id:namespaceId},
  ];
const version = () => ({
  id:versionId,annotations:{'workers/tag':sha},limits:{cpu_ms:10},urls:[],bindings:bindings(),
  env:Object.fromEntries(bindings().map(({name,type,class_name,namespace_id})=>[name,{type,...(class_name?{class_name,namespace_id}:{})}])),
  exports:{[DO_CLASS]:{type:'durable-object',storage:'sqlite',state:'created'}},
});
const deployment = () => ({id:deploymentId,versions:[{version_id:versionId,percentage:100}]});
const domain = (fields={}) => ({id:domainId,service:WORKER_NAME,hostname:'laptop-bridge-staging.security.cab',enabled:true,previews_enabled:false,zone_id:zoneId,...fields});
const ns = () => ({id:namespaceId,script:WORKER_NAME,class:DO_CLASS,use_sqlite:true});
const snapshot = () => ({
  zoneIds:[zoneId],worker:{...worker,references:{...worker.references,domains:[{id:domainId,hostname:'laptop-bridge-staging.security.cab',zone_id:zoneId}]}},workerId,subdomain:{enabled:false,previews_enabled:false},
  settings:{logpush:false,observability:{enabled:false},tail_consumers:[]},schedules:[],
  namespaces:[ns()],domains:[domain()],gatewayRules:[],version:version(),versions:[{id:versionId,metadata:{},number:1}],deployments:[deployment()],
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
  assert.match(workflow,/CF_BRIDGE_RELEASE_REVIEWER_IDS: \$\{\{ vars\.CF_BRIDGE_RELEASE_REVIEWER_IDS \}\}/);
  assert.match(workflow,/CF_BRIDGE_WORKER_ID: \$\{\{ vars\.CF_BRIDGE_WORKER_ID \}\}/);
  const gate=workflow.split('  deploy-disabled-bridge:',1)[0];
  assert.doesNotMatch(gate,/secrets\./);
  const provider=workflow.split('      - name: Read current provider state')[1].split('      - name: Deploy only')[0];
  assert.match(provider,/CF_BRIDGE_READ_TOKEN/);
  assert.match(provider,/CF_BRIDGE_WORKER_ID: \$\{\{ vars\.CF_BRIDGE_WORKER_ID \}\}/);
  assert.doesNotMatch(workflow,/CF_BRIDGE_BOOTSTRAP_WRITE_TOKEN|CF_BRIDGE_BOOTSTRAP_ACCOUNT_ID/);
  assert.match(workflow,/staging-bootstrap\.mjs assert-distinct-credentials/);
  assert.match(workflow,/cf build --mode bridge-init/);
  assert.match(workflow,/cf deploy --mode bridge-init --prebuilt --provision --tag "\$GITHUB_SHA"/);
  assert.match(workflow,/node staging-bootstrap\.mjs init-postflight/);
  assert.doesNotMatch(workflow,/cf deploy --mode bridge --prebuilt --provision/);
  assert.doesNotMatch(workflow,/CF_BRIDGE_ACCESS_ISSUER|CF_BRIDGE_ACCESS_AUDIENCE/);
  assert.doesNotMatch(workflow,/wrangler deploy|workers versions (create|deploy)|--dispatch-namespace/);
  for(const fixed of [WORKER_NAME,'CF_BRIDGE_ACCOUNT_ID','CF_BRIDGE_ZONE_IDS',ENVIRONMENT_NAME]) assert.ok(workflow.includes(fixed));
});

test('ordinary release gate verifies before dynamic environment assignment',async()=>{
  const workflow=await readFile(resolve('../../.github/workflows/private-laptop-bridge-release.yml'),'utf8');
  const [prejob,release]=workflow.split('  release:');
  assert.match(workflow,/verify-release-environment:/);
  assert.match(release,/needs: verify-release-environment/);
  assert.match(release,/name: \$\{\{ needs\.verify-release-environment\.outputs\.environment \}\}/);
  assert.doesNotMatch(release,/environment: cloudflare-staging/);
  assert.doesNotMatch(prejob,/secrets\./);
  assert.match(prejob,/CF_BRIDGE_RELEASE_REVIEWER_IDS: \$\{\{ vars\.CF_BRIDGE_RELEASE_REVIEWER_IDS \}\}/);
  assert.match(release,/CF_BRIDGE_ACCESS_ISSUER: \$\{\{ vars\.CF_BRIDGE_ACCESS_ISSUER \}\}/);
  assert.match(release,/CF_BRIDGE_ACCESS_AUDIENCE: \$\{\{ vars\.CF_BRIDGE_ACCESS_AUDIENCE \}\}/);
  assert.match(prejob,/staging-bootstrap\.mjs github-environment-release/);
});

function response(result, {status=200,resultInfo={page:1,per_page:100,total_count:Array.isArray(result)?result.length:0,total_pages:1},success=true,errors=[]}={}) {
  return new Response(JSON.stringify({success,result,errors,...(resultInfo === undefined ? {} : {result_info:resultInfo})}),{status});
}

function cloudflareFixture({target='absent', zones=[{id:zoneId,account:{id:accountId}}], zoneInfo, route=[], domains=[], namespaces=[], workerRecord=null, versionRecord=version(), gatewayRules=[], deployments=[deployment()], legacyVersions=null, overrides={}}={}) {
  const calls=[];
  const versionsList=legacyVersions??(deployments.length?[{id:versionId,metadata:{},number:1}]:[]);
  const effectiveWorkerRecord=workerRecord??{...worker,references:{...worker.references,domains:domains.filter(domain=>domain?.service===WORKER_NAME).map(domain=>({id:domain.id,hostname:domain.hostname,zone_id:domain.zone_id}))}};
  const fetchImpl=async (url,init={})=>{
    const parsed=new URL(url), method=init.method??'GET';
    calls.push({url:parsed,method,token:init.headers?.authorization});
    if(method!=='GET') return new Response('{}',{status:405});
    if(parsed.pathname==='/client/v4/zones') return response(zones,{resultInfo:zoneInfo??{page:1,per_page:50,total_count:zones.length,total_pages:zones.length?1:0}});
    if(parsed.pathname===`/client/v4/zones/${zoneId}/workers/routes`) return response(route,{resultInfo:undefined});
    if(parsed.pathname==='/client/v4/accounts/'+accountId+'/workers/domains') return response(domains);
    if(parsed.pathname==='/client/v4/accounts/'+accountId+'/gateway/rules') return response(gatewayRules);
    if(parsed.pathname==='/client/v4/accounts/'+accountId+'/workers/durable_objects/namespaces') return response(namespaces,{resultInfo:{page:1,per_page:1000,total_count:namespaces.length,total_pages:namespaces.length?1:0}});
    if(parsed.pathname===`/client/v4/accounts/${accountId}/workers/scripts/${WORKER_NAME}/versions`) return response({items:versionsList},{resultInfo:{page:1,per_page:100,count:versionsList.length,total_count:versionsList.length,total_pages:versionsList.length?1:0}});
    if(parsed.pathname===`/client/v4/accounts/${accountId}/workers/workers`) return response(target==='absent'?[]:[effectiveWorkerRecord],{resultInfo:{page:1,per_page:100,total_count:target==='absent'?0:1,total_pages:target==='absent'?0:1}});
    if(parsed.pathname===`/client/v4/accounts/${accountId}/workers/workers/${workerId}`) {
      if(target==='absent') return response(null,{status:404,success:false,resultInfo:undefined,errors:[{code:10007}]});
      return response(effectiveWorkerRecord,{resultInfo:undefined});
    }
    if(parsed.pathname.endsWith(`/workers/scripts/${WORKER_NAME}/subdomain`)) return response({enabled:false,previews_enabled:false});
    if(parsed.pathname.endsWith(`/workers/scripts/${WORKER_NAME}/script-settings`)) return response({logpush:false,observability:{enabled:false},tail_consumers:[]});
    if(parsed.pathname.endsWith(`/workers/scripts/${WORKER_NAME}/schedules`)) return response({schedules:[]});
    if(parsed.pathname.endsWith(`/workers/scripts/${WORKER_NAME}/deployments`)) return response({deployments});
    if(parsed.pathname.endsWith(`/workers/workers/${workerId}/versions/${versionId}`)) return response(versionRecord);
    const replacement=overrides[parsed.pathname];
    if(replacement) return replacement;
    throw new Error(`unexpected synthetic request ${method} ${parsed.pathname}`);
  };
  return {fetchImpl,calls};
}

test('GitHub dispatch gate proves no admin bypass, typed independent reviewers, and branch-only main',()=>{
  const environment={name:ENVIRONMENT_NAME,can_admins_bypass:false,protection_rules:[{type:'required_reviewers',prevent_self_review:true,reviewers:[{type:'User',reviewer:{id:71}}]}],deployment_branch_policy:{protected_branches:false,custom_branch_policies:true}};
  const branchPolicies={total_count:1,branch_policies:[{name:'main',type:'branch'}]};
  const expectedReviewerIds='[{"type":"User","id":71}]';
  assert.deepEqual(verifyEnvironmentProtection({environment,branchPolicies,expectedReviewerIds,dispatcherId:99}),{environment:ENVIRONMENT_NAME});
  assert.throws(()=>verifyEnvironmentProtection({environment:{...environment,can_admins_bypass:true},branchPolicies,expectedReviewerIds,dispatcherId:99}),/administrator bypass/);
  assert.throws(()=>verifyEnvironmentProtection({environment:{...environment,can_admins_bypass:undefined},branchPolicies,expectedReviewerIds,dispatcherId:99}),/did not prove administrator bypass/);
  assert.throws(()=>verifyEnvironmentProtection({environment,branchPolicies:{total_count:1,branch_policies:[{name:'main'}]},expectedReviewerIds,dispatcherId:99}),/branch-only main/);
  assert.throws(()=>verifyEnvironmentProtection({environment,branchPolicies:{total_count:1,branch_policies:[{name:'main',type:'tag'}]},expectedReviewerIds,dispatcherId:99}),/branch-only main/);
  assert.throws(()=>verifyEnvironmentProtection({environment,branchPolicies,expectedReviewerIds:'[{"type":"User","id":71}]',dispatcherId:71}),/only eligible/);
  for(const change of [
    {environment:null},
    {environment:{...environment,name:'cloudflare-staging-other'}},
    {environment:{...environment,protection_rules:[{type:'required_reviewers',prevent_self_review:false,reviewers:[{type:'User',reviewer:{id:71}}]}]}},
    {environment:{...environment,protection_rules:[{type:'required_reviewers',prevent_self_review:true,reviewers:[{type:'User',reviewer:{id:72}}]}]}},
    {environment:{...environment,protection_rules:[{type:'required_reviewers',prevent_self_review:true,reviewers:[{type:'User',reviewer:{id:71}},{type:'Team',reviewer:{id:72}}]}]}},
    {branchPolicies:{total_count:2,branch_policies:[{name:'main'},{name:'release/*'}]}},
    {branchPolicies:{total_count:1,branch_policies:[{name:'*'}]}},
  ]) assert.throws(()=>verifyEnvironmentProtection({environment:Object.hasOwn(change,'environment')?change.environment:environment,branchPolicies:change.branchPolicies??branchPolicies,expectedReviewerIds,dispatcherId:99}));
  assert.throws(()=>verifyEnvironmentProtection({environment,branchPolicies,expectedReviewerIds:'',dispatcherId:99}));
});

test('exact empty locally created Worker is the only first-deploy candidate; all provider calls are GET',async()=>{
  const f=cloudflareFixture({target:'empty',deployments:[],domains:[]});
  const status=await verifyBootstrapPreflight({accountId,workerId,readToken,gatewayToken,expectedZoneIds:[zoneId],sourceSha:sha,fetchImpl:f.fetchImpl});
  assert.equal(status,'deploy');
  assert.ok(f.calls.every(call=>call.method==='GET'&&call.token===`Bearer ${call.url.pathname.endsWith('/gateway/rules')?gatewayToken:readToken}`));
  assert.ok(f.calls.some(call=>call.url.pathname===`/client/v4/zones/${zoneId}/workers/routes`));
  assert.ok(f.calls.some(call=>call.url.pathname===`/client/v4/accounts/${accountId}/workers/scripts/${WORKER_NAME}/versions`));
  await assert.rejects(verifyBootstrapPreflight({accountId,workerId,readToken,gatewayToken,expectedZoneIds:[zoneId],sourceSha:sha,fetchImpl:cloudflareFixture({target:'absent'}).fetchImpl}),/owner-created Worker is missing/);
  for(const bad of [
    cloudflareFixture({target:'absent',zones:[]}),
    cloudflareFixture({target:'absent',route:[{id:'route',pattern:'*',script:WORKER_NAME}]}),
    cloudflareFixture({target:'absent',domains:[{service:WORKER_NAME,hostname:'unsafe.example'}]}),
    cloudflareFixture({target:'absent',namespaces:[ns()]}),
    cloudflareFixture({target:'absent',zoneInfo:{page:2,per_page:50,total_count:1,total_pages:1}}),
    cloudflareFixture({target:'absent',gatewayRules:[{action:'forward',enabled:true,rule_settings:{forward:{worker_id:WORKER_NAME}}}]}),
  ]) await assert.rejects(verifyBootstrapPreflight({accountId,workerId,readToken,gatewayToken,expectedZoneIds:[zoneId],sourceSha:sha,fetchImpl:bad.fetchImpl}));
  const unservedVersion=cloudflareFixture({target:'empty',deployments:[],legacyVersions:[{id:versionId,metadata:{},number:1}]});
  await assert.rejects(verifyBootstrapPreflight({accountId,workerId,readToken,gatewayToken,expectedZoneIds:[zoneId],sourceSha:sha,fetchImpl:unservedVersion.fetchImpl}),/only the exact empty disabled locally created Worker/);
  const takeover=cloudflareFixture({target:'empty',deployments:[],domains:[],workerRecord:{...worker,id:'4'.repeat(32)}});
  await assert.rejects(verifyBootstrapPreflight({accountId,workerId,readToken,gatewayToken,expectedZoneIds:[zoneId],sourceSha:sha,fetchImpl:takeover.fetchImpl}),/pinned Beta identity/);
  assert.equal(verifyDistinctWriteCredential(readToken,gatewayToken,writeToken),true);
  assert.throws(()=>verifyDistinctWriteCredential(readToken,gatewayToken,gatewayToken),/separate/);
});

test('bootstrap only accepts disabled no-ingress state; routed postflight verifies the Access-host phase',()=>{
  const good=snapshot();
  assert.throws(()=>verifyProviderState(good,{phase:'preflight',expectedZoneIds:[zoneId],sourceSha:sha,expectedWorkerId:workerId}),/reverse domain references/);
  assert.equal(verifyProviderState(good,{phase:'postflight',expectedZoneIds:[zoneId],sourceSha:sha,expectedWorkerId:workerId}),'verified');
  const noIngress=snapshot();noIngress.domains=[];noIngress.worker.references.domains=[];
  assert.equal(verifyProviderState(noIngress,{phase:'preflight',expectedZoneIds:[zoneId],sourceSha:sha,expectedWorkerId:workerId}),'already-initialized-no-ingress');
  assert.equal(verifyProviderState(noIngress,{phase:'init-postflight',expectedZoneIds:[zoneId],sourceSha:sha,expectedWorkerId:workerId}),'verified-no-ingress');
  for(const kind of ['workers','durable_objects','dispatch_namespace_outbounds','queues']) {
    const referenced=structuredClone(noIngress);referenced.worker.references[kind]=[{id:workerId,name:'unexpected-worker'}];
    assert.throws(()=>verifyProviderState(referenced,{phase:'preflight',expectedZoneIds:[zoneId],sourceSha:sha,expectedWorkerId:workerId}),new RegExp(`reverse ${kind}`));
  }
  const incompleteRefs=structuredClone(noIngress);delete incompleteRefs.worker.references.queues;
  assert.throws(()=>verifyProviderState(incompleteRefs,{phase:'preflight',expectedZoneIds:[zoneId],sourceSha:sha,expectedWorkerId:workerId}),/reverse references or telemetry state are missing/);
  const wrongBetaId=structuredClone(noIngress);wrongBetaId.worker.id='4'.repeat(32);
  assert.throws(()=>verifyProviderState(wrongBetaId,{phase:'preflight',expectedZoneIds:[zoneId],sourceSha:sha,expectedWorkerId:workerId}),/immutable ID does not match/);
  const unexpectedDomain=structuredClone(noIngress);unexpectedDomain.worker.references.domains=[{id:'33333333-4444-5555-6666-777777777777',hostname:'other.example',zone_id:zoneId}];
  assert.throws(()=>verifyProviderState(unexpectedDomain,{phase:'init-postflight',expectedZoneIds:[zoneId],sourceSha:sha,expectedWorkerId:workerId}),/reverse domain references/);
  const extraVersion=structuredClone(noIngress);extraVersion.versions.push({id:'22222222-3333-4444-5555-666666666666',metadata:{},number:2});
  assert.throws(()=>verifyProviderState(extraVersion,{phase:'init-postflight',expectedZoneIds:[zoneId],sourceSha:sha,expectedWorkerId:workerId}),/exactly one version/);
  assert.throws(()=>verifyProviderState(good,{phase:'init-postflight',expectedZoneIds:[zoneId],sourceSha:sha,expectedWorkerId:workerId}),/reverse domain references/);
  const reverseRef=snapshot();reverseRef.domains=[];reverseRef.worker=structuredClone(worker);reverseRef.worker.references.domains=[{id:domainId,hostname:'laptop-bridge-staging.security.cab',zone_id:zoneId}];
  assert.throws(()=>verifyProviderState(reverseRef,{phase:'init-postflight',expectedZoneIds:[zoneId],sourceSha:sha,expectedWorkerId:workerId}),/reverse domain references/);
  assert.throws(()=>verifyProviderState(noIngress,{phase:'preflight',expectedZoneIds:[zoneId],sourceSha:'c'.repeat(40),expectedWorkerId:workerId}),/exact reviewed source SHA/);
  for(const mutate of [
    s=>{s.zoneIds=[];},
    s=>{s.subdomain.enabled=true;},
    s=>{s.subdomain.previews_enabled=true;},
    s=>{s.schedules=[{cron:'* * * * *'}];},
    s=>{s.settings.logpush=true;},
    s=>{s.settings.tail_consumers=[{}];},
    s=>{s.domains=[domain({hostname:'unsafe.example'})];},
    s=>{s.domains=[domain({service:'other-worker'})];},
    s=>{s.domains=[domain({previews_enabled:true})];},
    s=>{s.domains=[domain({enabled:false})];},
    s=>{s.deployments=[];},
    s=>{s.namespaces[0].script='other-worker';},
    s=>{s.namespaces[0].use_sqlite=false;},
    s=>{s.deployments[0].versions[0].percentage=50;},
    s=>{s.version.bindings[0].text='true';},
    s=>{s.version.bindings.push({name:'OUTSIDE',type:'service'});},
    s=>{s.version.exports[DO_CLASS].storage='legacy-kv';},
    s=>{s.version.limits.cpu_ms=50;},
    s=>{s.version.env.ENABLED.text='true';},
    s=>{s.version.urls=['https://preview.example'];},
  ]) { const bad=snapshot(); mutate(bad); assert.throws(()=>verifyProviderState(bad,{phase:'preflight',expectedZoneIds:[zoneId],sourceSha:sha,expectedWorkerId:workerId})); }
  assert.throws(()=>verifyProviderState(good,{phase:'postflight',expectedZoneIds:[zoneId],sourceSha:'c'.repeat(40),expectedWorkerId:workerId}),/exact reviewed source SHA/);
});

test('postflight readback verifies deployed identity via independent API data and never writes',async()=>{
  const f=cloudflareFixture({target:'existing',namespaces:[ns()],domains:[domain()]});
  assert.equal(await verifyBootstrapPostflight({accountId,workerId,readToken,gatewayToken,expectedZoneIds:[zoneId],sourceSha:sha,fetchImpl:f.fetchImpl}),'verified');
  const init=cloudflareFixture({target:'existing',namespaces:[ns()],domains:[]});
  assert.equal(await verifyBootstrapPostflight({accountId,workerId,readToken,gatewayToken,expectedZoneIds:[zoneId],sourceSha:sha,phase:'init-postflight',fetchImpl:init.fetchImpl}),'verified-no-ingress');
  assert.ok(f.calls.every(call=>call.method==='GET'&&call.token===`Bearer ${call.url.pathname.endsWith('/gateway/rules')?gatewayToken:readToken}`));
  const bad=cloudflareFixture({target:'existing',namespaces:[ns()],domains:[domain()],versionRecord:{...version(),annotations:{'workers/tag':'c'.repeat(40)}}});
  await assert.rejects(verifyBootstrapPostflight({accountId,workerId,readToken,gatewayToken,expectedZoneIds:[zoneId],sourceSha:sha,fetchImpl:bad.fetchImpl}),/exact reviewed source SHA/);
});
