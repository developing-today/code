// Ephemeral loopback browser test harness; never proxies a live environment.
// Start: node tests/local-server.mjs
import {DatabaseSync} from 'node:sqlite';
import {readFileSync} from 'node:fs';
import {createServer} from 'node:http';
import {assets,assetType} from '../scripts/assets.mjs';
import {api} from '../worker/api.mjs';
if(process.argv.length!==2)throw Error('Usage: node tests/local-server.mjs');
const sql=new DatabaseSync(':memory:');
for(const file of ['0000_tracker.sql','0001_store_defaults.sql'])sql.exec(readFileSync('drizzle/'+file,'utf8'));
const db={
 prepare(q){let v=[];return {bind(...args){v=args;return this},first(){return sql.prepare(q).get(...v)||null},all(){return {results:sql.prepare(q).all(...v)}},run(){return {meta:{changes:Number(sql.prepare(q).run(...v).changes)}}}}},
 batch(stmts){sql.exec('BEGIN IMMEDIATE');try{const r=stmts.map(s=>s.run());sql.exec('COMMIT');return r}catch(e){sql.exec('ROLLBACK');throw e}}
};
const server=createServer(async(req,res)=>{
 try{
  const origin='http://127.0.0.1:'+server.address().port,u=new URL(req.url,origin);
  if(u.pathname.startsWith('/api/')){
   const chunks=[];for await(const x of req)chunks.push(x);
   const h=new Headers({'oai-authenticated-user-id':'fixture-user','oai-authenticated-user-email':'fixture@example.test'});
   for(const x of ['origin','content-type','sec-fetch-site'])if(req.headers[x])h.set(x,req.headers[x]);
   const r=await api(new Request(u,{method:req.method,headers:h,...(req.method==='POST'?{body:Buffer.concat(chunks)}:{})}),{DB:db});
   res.writeHead(r.status,Object.fromEntries(r.headers));res.end(Buffer.from(await r.arrayBuffer()));return;
  }
  if(!Object.hasOwn(assets,u.pathname)){res.writeHead(404);res.end();return;}
  const f=assets[u.pathname];res.writeHead(200,{'content-type':assetType(f)});res.end(readFileSync('public/'+f));
 }catch{res.writeHead(500);res.end('Local test harness error');}
});
server.listen(0,'127.0.0.1',()=>console.log('http://127.0.0.1:'+server.address().port));
