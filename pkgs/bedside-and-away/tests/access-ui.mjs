import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const ctx={window:{},console};vm.createContext(ctx);vm.runInContext(readFileSync(new URL('../public/access-ui.js',import.meta.url),'utf8'),ctx);
const ui=ctx.window.BedawayAccessUI;
assert.equal(ui.parseDraft('{"schemaVersion":1,"groups":{},"policies":{}}').schemaVersion,1);
for(const text of ['[]','null','{"schemaVersion":2,"groups":{},"policies":{}}','{"schemaVersion":1,"groups":[],"policies":{}}'])assert.throws(()=>ui.parseDraft(text));
const d=ui.parseDraft('{"schemaVersion":1,"groups":{},"policies":{}}');
ui.putGroup(d,'team',['alice','alice','bob']);assert.equal(d.groups.team.length,2);
ui.putPolicy(d,'item:one',{rules:[]});assert.ok(d.policies['item:one']);
assert.throws(()=>ui.putGroup(d,'__proto__',['x']));
assert.throws(()=>ui.putPolicy(d,'constructor',{rules:[]}));
ui.removePolicy(d,'item:one');assert.equal(Object.keys(d.policies).length,0);
ui.putPolicy(d,'item:two',{rules:[{effect:'allow',subject:{type:'group',id:'team'},actions:['read']}]});
assert.throws(()=>ui.removeGroup(d,'team'));
assert.equal(ui.canManage({role:'reader',storeId:'s'}),false);assert.equal(ui.canManage({role:'admin',storeId:'s'}),true);
console.log('PASS admin UI draft parsing, keyed CRUD, reference protection and role gate');
class Element{
 constructor(tag){this.tag=tag;this.children=[];this.listeners={};this.attrs={};this.textContent='';this.value='';this.style={};this.checked=false;this.isConnected=true;}
 setAttribute(k,v){this.attrs[k]=v} append(...x){this.children.push(...x)} replaceChildren(...x){this.children=x} addEventListener(e,fn){this.listeners[e]=fn} remove(){this.isConnected=false} showModal(){} close(){this.listeners.close?.()} checkValidity(){return true}
}
const document={body:new Element('body'),createElement:tag=>new Element(tag),addEventListener(){}};
const uiCtx={window:{confirm:()=>true},document,crypto:{randomUUID:()=> 'fixed-command-id'},console};vm.createContext(uiCtx);vm.runInContext(readFileSync(new URL('../public/access-ui.js',import.meta.url),'utf8'),uiCtx);
const all=n=>[n,...n.children.flatMap(all)];let context={storeId:'store',role:'admin'};const requests=[];let reloaded=0;
await uiCtx.window.BedawayAccessUI.open({getContext:()=>context,request:async(path,data)=>{requests.push({path,data});return data?{version:3}:{version:2,members:[],access:{schemaVersion:1,groups:{},policies:{}}}},reload:async()=>{reloaded++}});
let dialog=document.body.children.at(-1);const find=text=>all(dialog).find(n=>n.tag==='button'&&n.textContent===text);
find('Review policy changes').listeners.click();const consent=all(dialog).find(n=>n.tag==='input'&&n.attrs.type==='checkbox');consent.checked=true;
find('Apply reviewed policy').listeners.click();await new Promise(r=>setTimeout(r,0));
assert.equal(requests.filter(r=>r.data).length,1);assert.equal(requests.find(r=>r.data).data.commandId,'fixed-command-id');assert.equal(requests.find(r=>r.data).data.expectedVersion,2);assert.equal(reloaded,1);
find('Review policy changes').listeners.click();context={storeId:'other',role:'admin'};all(dialog).find(n=>n.tag==='input'&&n.attrs.type==='checkbox').checked=true;find('Apply reviewed policy').listeners.click();await new Promise(r=>setTimeout(r,0));assert.equal(requests.filter(r=>r.data).length,1);
await assert.rejects(()=>uiCtx.window.BedawayAccessUI.open({getContext:()=>({role:'reader',storeId:'s'}),request:async()=>{throw Error('must not call')},reload:async()=>{}}));
console.log('PASS admin dialog load/review/confirmed save, command ID, expected revision, switched-store guard and reader denial');
