// BA-W01 L4: model-based and property tests for the §4 evaluator, the policy write gate, the query
// scope builder and shadow mode. The model is the contract's ordered rules, written independently.
import assert from 'node:assert/strict';
import {evaluateAccess,assertPolicyWrite,queryScope,enforcementMode,guard,PRIORITY_OVERRIDES} from '../worker/access.mjs';

assert.equal(PRIORITY_OVERRIDES,false,'contract §11 default: no numeric overrides until reviewed');

let seed=0x9e3779b9;const rand=n=>{seed^=seed<<13;seed^=seed>>>17;seed^=seed<<5;return (seed>>>0)%n};
const pick=xs=>xs[rand(xs.length)];
const users=['alice','bob','carol'],acts=['read','write','admin'],levels={reader:1,editor:2,admin:3},need={read:1,write:2,admin:3};

function generate(){
 const groups={g1:users.filter(()=>rand(2)),g2:users.filter(()=>rand(2))};
 const keys=['*','item:a','item:b','tag:t'],policies={};
 for(const k of keys){if(!rand(3))continue;const rules=[];for(let i=rand(4);i--;){
  const effect=pick(['allow','deny']),type=pick(['everyone','user','group']);
  const r={effect,subject:type==='everyone'?{type}:{type,id:type==='user'?pick(users):pick(['g1','g2'])},actions:[...new Set([pick(acts),pick(acts)])],priority:pick([0,50,100,100,150])};
  if(effect==='allow'&&rand(2)){r.overrideDeny=true;if(rand(3))r.authority=pick(['admin','service'])}
  rules.push(r)}
  policies[k]={rules,...(rand(4)?{}:{default:pick(['allow','deny'])})};
 }
 // Inheritance only points "down" the key order, so it is acyclic by construction.
 if(policies['item:a']&&policies['tag:t']&&rand(2))policies['item:a'].inherit=['tag:t'];
 return {schemaVersion:1,groups,policies};
}

// Independent model of §4 step 4.
function model({principal,role,access,entityId,action,priorityOverrides}){
 if(!principal?.id||!(role in levels)||levels[role]<need[action])return false;
 if(!access)return true;
 const keys=new Set(),add=k=>{if(keys.has(k)||!access.policies[k])return;keys.add(k);(access.policies[k].inherit||[]).forEach(add)};add('*');add(entityId);
 const hit=[...keys].flatMap(k=>access.policies[k].rules).filter(r=>r.actions.includes(action)&&(r.subject.type==='everyone'||r.subject.type==='user'&&r.subject.id===principal.id||r.subject.type==='group'&&access.groups[r.subject.id].includes(principal.id)));
 const denies=hit.filter(r=>r.effect==='deny'),allows=hit.filter(r=>r.effect==='allow');
 if(denies.length){
  if(!priorityOverrides)return false;
  const top=Math.max(...denies.map(r=>r.priority??100));
  return allows.some(r=>r.overrideDeny&&['admin','service'].includes(r.authority)&&(r.priority??100)>top);
 }
 const defaultDeny=[...keys].some(k=>access.policies[k].default==='deny');
 return !(defaultDeny&&!allows.length);
}

let checked=0,denied=0;
for(let n=0;n<3000;n++){
 const access=generate();
 for(const priorityOverrides of [false,true])for(const role of ['reader','editor','admin'])for(const user of users)for(const entityId of ['item:a','item:b','tag:t'])for(const action of acts){
  const c={principal:{id:user},role,access,entityId,action,priorityOverrides};
  const got=evaluateAccess(c).allowed,want=model(c);
  if(got!==want)assert.fail('evaluator disagrees with §4 model: '+JSON.stringify({c,got,want}));
  checked++;if(!got)denied++;
  // Default build ignores overrides entirely: any matching deny is a hard block.
  if(!priorityOverrides&&got)assert.equal(evaluateAccess({...c,priorityOverrides:true}).allowed,true,'enabling overrides can only add allows');
 }
}
assert(denied>0&&denied<checked,'generator exercises both outcomes');

// §8 test 3: equal-priority allow vs deny resolves to deny, whatever the flags.
const tie={schemaVersion:1,groups:{},policies:{'item:x':{rules:[{effect:'deny',subject:{type:'everyone'},actions:['read'],priority:100},{effect:'allow',subject:{type:'user',id:'alice'},actions:['read'],priority:100,overrideDeny:true,authority:'admin'}]}}};
for(const priorityOverrides of [false,true])assert.equal(evaluateAccess({principal:{id:'alice'},role:'admin',access:tie,entityId:'item:x',priorityOverrides}).allowed,false);

// §8 test 2: an editor cannot raise a priority to defeat an inherited deny. The write is rejected and
// the evaluator still denies.
const escape={schemaVersion:1,groups:{},policies:{'tag:t':{rules:[{effect:'deny',subject:{type:'everyone'},actions:['read'],priority:100}]},'item:x':{inherit:['tag:t'],rules:[{effect:'allow',subject:{type:'user',id:'bob'},actions:['read'],priority:1000000,overrideDeny:true}]}}};
assert.throws(()=>assertPolicyWrite(escape,{role:'editor'}),/Administrator required/);
assert.throws(()=>assertPolicyWrite(escape,{role:'admin'}),/Priority overrides are disabled/);
for(const priorityOverrides of [false,true])assert.equal(evaluateAccess({principal:{id:'bob'},role:'editor',access:escape,entityId:'item:x',priorityOverrides}).allowed,false,'unstamped rule never overrides');
assert.throws(()=>assertPolicyWrite({...escape,policies:{...escape.policies,'item:x':{rules:[{effect:'allow',subject:{type:'everyone'},actions:['read'],authority:'editor'}]}}},{role:'admin'}),/Invalid policy rule/);
// With overrides enabled after review, the endpoint stamps the writer's authority; clients cannot.
const stamped=assertPolicyWrite(escape,{role:'admin',priorityOverrides:true});
assert.equal(stamped.policies['item:x'].rules[0].authority,'admin');
assert.equal(escape.policies['item:x'].rules[0].authority,undefined,'input is not mutated');
const forged=structuredClone(tie);forged.policies['item:x'].rules[1].overrideDeny=false;
assert.equal(assertPolicyWrite(forged,{role:'admin'}).policies['item:x'].rules[1].authority,undefined,'authority is never kept from client input');

// §5: search and counts go through one scope and are computed after the filter.
const state={items:[{id:'a',name:'Lamp',tags:[],attributes:[]},{id:'b',name:'Secret lamp',tags:[],attributes:[]},{id:'c',name:'Chair',tags:[],attributes:[]}],tags:[],notes:[],inventory:[],
 access:{schemaVersion:1,groups:{},policies:{'item:b':{rules:[{effect:'deny',subject:{type:'user',id:'bob'},actions:['read']}]}}}};
const lamp=x=>/lamp/i.test(x.name);
const bob=queryScope(state,{principal:{id:'bob'},role:'reader'}),alice=queryScope(state,{principal:{id:'alice'},role:'reader'});
assert.deepEqual(bob.search('items',lamp).rows.map(x=>x.id),['a']);assert.equal(bob.count('items',lamp),1);assert.equal(bob.count('items'),2);
assert.deepEqual(alice.search('items',lamp).rows.map(x=>x.id),['a','b']);assert.equal(alice.count('items',lamp),2);
assert.deepEqual(alice.search('items',()=>true,1),{rows:[alice.view.items[0]],truncated:true});
assert.equal(queryScope(state,{principal:{id:'eve'},role:null}).count('items'),0,'nonmember sees no rows and a zero count');
assert.equal(JSON.stringify(bob.view).includes('Secret'),false);

// §9: shadow mode per surface; surfaces that already enforce cannot be switched back.
const env={ACCESS_SHADOW_SURFACES:'history, exports,search,api'};
assert.equal(enforcementMode(env,'history'),'shadow');assert.equal(enforcementMode(env,'exports'),'shadow');
assert.equal(enforcementMode(env,'media'),'enforce');assert.equal(enforcementMode(env,'search'),'enforce');assert.equal(enforcementMode(env,'api'),'enforce');assert.equal(enforcementMode({},'history'),'enforce');
assert.throws(()=>enforcementMode(env,'nope'),/Unknown access surface/);
const logs=[],no={allowed:false,reason:'inherited-deny'};
const shadowed=guard({env,surface:'history',log:x=>logs.push(x)},no);
assert.equal(shadowed.allowed,true);assert.equal(shadowed.wouldAllow,false);assert.equal(shadowed.enforced,false);assert.equal(logs.length,1);assert.match(logs[0],/inherited-deny/);
const enforced=guard({env,surface:'media',log:x=>logs.push(x)},no);
assert.equal(enforced.allowed,false);assert.equal(enforced.enforced,true);assert.equal(logs.length,1,'enforced surfaces do not log as shadow');
assert.equal(guard({env,surface:'search',log:x=>logs.push(x)},no).allowed,false);

console.log(`PASS policy evaluator: ${checked} model-checked decisions, equal-priority deny, editor cannot escape an inherited deny, overrides off by default, query scope counts after filter, per-surface shadow mode.`);
