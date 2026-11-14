// Contract: node external/migrate.mjs [--apply]. Credentials are environment-only.
const args=process.argv.slice(2);
if(args.length>1||args.length===1&&args[0]!=='--apply')throw Error('Usage: node external/migrate.mjs [--apply]');
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {createTurso} from './turso.mjs';
const files=['../drizzle/0000_tracker.sql','../drizzle/0001_store_defaults.sql','0002_gateway.sql','0003_steering_outbox.sql'];
const planned=files.map(path=>{const text=readFileSync(new URL(path,import.meta.url),'utf8');return {name:path.split('/').at(-1),hash:createHash('sha256').update(text).digest('hex'),text}});
if(!args.length){
 console.log('Plan only. Schema only, no existing user data migration. Use --apply after destination and backup review.');
 for(const p of planned)console.log(p.name+' '+p.hash);
 process.exit(0);
}
if(!process.env.TURSO_URL||!process.env.TURSO_AUTH_TOKEN)throw Error('Set dedicated TURSO_URL and TURSO_AUTH_TOKEN in the process environment.');
const db=createTurso({url:process.env.TURSO_URL,token:process.env.TURSO_AUTH_TOKEN});
await db.prepare('CREATE TABLE IF NOT EXISTS bedaway_schema_migrations (name TEXT PRIMARY KEY NOT NULL, sha256 TEXT NOT NULL, applied_at TEXT NOT NULL)').run();
for(const p of planned){
 const existing=await db.prepare('SELECT sha256 FROM bedaway_schema_migrations WHERE name=?').bind(p.name).first();
 if(existing){if(existing.sha256!==p.hash)throw Error('Applied migration changed: '+p.name);console.log('Verified '+p.name);continue}
 const parts=p.name==='0002_gateway.sql'||p.name==='0003_steering_outbox.sql'?p.text.split(';').filter(s=>s.trim()):p.text.split('--> statement-breakpoint').filter(s=>s.trim());
 await db.batch([...parts.map(s=>db.prepare(s)),db.prepare('INSERT INTO bedaway_schema_migrations(name,sha256,applied_at) VALUES(?,?,?)').bind(p.name,p.hash,new Date().toISOString())]);
 console.log('Applied '+p.name);
}
console.log('Schema ready. Verify authenticated live operations and data migration before switching traffic.');
