// Minimal D1-shaped adapter over Hrana HTTP v2. No retries after uncertain writes.
function arg(value){
 if(value===null)return {type:'null'};
 if(typeof value==='string')return {type:'text',value};
 if(typeof value==='boolean')return {type:'integer',value:value?'1':'0'};
 if(typeof value==='bigint')return {type:'integer',value:String(value)};
 if(typeof value==='number'&&Number.isFinite(value))return Number.isInteger(value)?{type:'integer',value:String(value)}:{type:'float',value};
 throw Error('Unsupported SQL binding');
}
function value(v){if(v.type==='null')return null;if(v.type==='integer'){const n=Number(v.value);if(!Number.isSafeInteger(n))throw Error('Unsafe SQL integer');return n}if(v.type==='float')return v.value;if(v.type==='text')return v.value;throw Error('Unsupported SQL result type')}
function result(r){if(!r||!Array.isArray(r.cols)||!Array.isArray(r.rows))throw Error('Invalid database result');return {results:r.rows.map(row=>Object.fromEntries(r.cols.map((c,i)=>[c.name,value(row[i])]))),meta:{changes:Number(r.affected_row_count||0)},success:true}}
export function atomicBatch(statements){
 const steps=[{stmt:{sql:'BEGIN IMMEDIATE',want_rows:false}}];
 for(const stmt of statements)steps.push({condition:{type:'ok',step:steps.length-1},stmt});
 const commit=steps.length;steps.push({condition:{type:'ok',step:commit-1},stmt:{sql:'COMMIT',want_rows:false}});
 steps.push({condition:{type:'not',cond:{type:'ok',step:commit}},stmt:{sql:'ROLLBACK',want_rows:false}});
 return {steps,commit};
}
export function createTurso({url,token,fetcher=(...args)=>globalThis.fetch(...args)}){
 const endpoint=new URL(String(url).replace(/^libsql:/,'https:'));
 if(endpoint.protocol!=='https:'||endpoint.username||endpoint.password||!token)throw Error('Secure Turso configuration required');
 endpoint.pathname='/v2/pipeline';endpoint.search='';endpoint.hash='';
 async function pipeline(request){
  let res;try{res=await fetcher(endpoint.href,{method:'POST',redirect:'manual',headers:{Authorization:'Bearer '+token,'content-type':'application/json'},body:JSON.stringify({requests:[request,{type:'close'}]})});}catch{throw Object.assign(Error('Database transport failed; outcome may be unknown'),{phase:'database-transport'})}
  if(!res.ok)throw Object.assign(Error('Database request failed; outcome may be unknown'),{phase:'database-http-'+res.status});
  const payload=await res.json(),first=payload.results?.[0];
  if(first?.type!=='ok'||first.response?.type!==request.type)throw Object.assign(Error('Database operation failed'),{phase:'database-protocol'});
  return first.response.result;
 }
 function prepare(sql){let args=[];const statement={
  bind(...values){args=values.map(arg);return statement},
  _stmt(){return {sql,args,want_rows:true}},
  async all(){return result(await pipeline({type:'execute',stmt:statement._stmt()}))},
  async first(){return (await statement.all()).results[0]??null},
  async run(){return statement.all()}
 };return statement}
 return {prepare,async batch(statements){
  if(!statements.length)return [];
  const {steps,commit}=atomicBatch(statements.map(s=>s._stmt())),r=await pipeline({type:'batch',batch:{steps}});
  if(!r||r.step_errors?.some(Boolean)||!r.step_results?.[commit])throw Object.assign(Error('Atomic database batch failed'),{phase:'database-batch'});
  return r.step_results.slice(1,commit).map(result);
 }};
}
