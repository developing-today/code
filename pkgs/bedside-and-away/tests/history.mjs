import assert from 'node:assert/strict';
import {historyTransition} from '../worker/history.mjs';
const before={items:[{id:'a',name:'A',qty:1},{id:'b',name:'B',qty:1}],notes:[]};
const after=structuredClone(before);after.items[0].qty=2;
const event={id:'original',operation:'change',before,after};
const current=structuredClone(after);current.items[1].name='B edited later';
const undo=historyTransition({current,event,action:'undo',latest:null});
assert.equal(undo.items[0].qty,1);assert.equal(undo.items[1].name,'B edited later');
const redo=historyTransition({current:undo,event,action:'redo',latest:{operation:'history.undo'}});
assert.deepEqual(redo,current);
assert.throws(()=>historyTransition({current:{...current,items:[{id:'a',name:'A',qty:99},current.items[1]]},event,action:'undo'}),/conflict/i);
assert.throws(()=>historyTransition({current,event,action:'undo',latest:{operation:'history.undo'}}),/already undone/i);
assert.throws(()=>historyTransition({current,event,action:'redo'}),/undo first/i);
const group={id:'group',kind:'virtual',name:'Group',parent:''};
const grouped={...after,items:[{...after.items[0],parent:'group'},after.items[1],group]};
const groupEvent={id:'grouping',operation:'variants',before:after,after:grouped};
assert.deepEqual(historyTransition({current:grouped,event:groupEvent,action:'undo'}),after);
const changedGroup=structuredClone(grouped);changedGroup.items[2].name='Edited';
assert.throws(()=>historyTransition({current:changedGroup,event:groupEvent,action:'undo'}),/conflict/i);
const restored=historyTransition({current:after,event:groupEvent,action:'redo',latest:{operation:'history.undo'}});assert.deepEqual(restored,grouped);
assert.throws(()=>historyTransition({current,event:{...event,operation:'steering'},action:'undo'}),/unsupported/i);
assert.throws(()=>historyTransition({current,event:{...event,after:{...after,access:{}}},action:'undo'}),/protected/i);
console.log('PASS history: inverse patches, unrelated edits, conflicts, lifecycle, variant IDs and protected state.');
// Property oracle: inverse then forward preserves unrelated records, for 1,000 generated edits.
for(let i=0;i<1000;i++){
 const b={items:[{id:'x',qty:i},{id:'y',qty:i+1}]},a=structuredClone(b);a.items[0].qty=i+3;
 const c=structuredClone(a);c.items[1].qty=i+99;
 const e={operation:'change',before:b,after:a};
 const u=historyTransition({current:c,event:e,action:'undo'});
 assert.equal(u.items[0].qty,i);assert.equal(u.items[1].qty,i+99);
 assert.deepEqual(historyTransition({current:u,event:e,action:'redo',latest:{operation:'history.undo'}}),c);
}
console.log('PASS: 1,000 inverse/redo noninterference cases.');
