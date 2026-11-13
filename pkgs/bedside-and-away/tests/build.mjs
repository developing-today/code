import assert from 'node:assert/strict';
import {execFileSync,spawnSync} from 'node:child_process';
import {readFileSync,readdirSync} from 'node:fs';
import {assets} from '../scripts/assets.mjs';
const env={...process.env,TURSO_URL:'https://database.test',GATEWAY_ORIGIN:'https://gateway.example.ts.net',GATEWAY_HMAC_KEY:'DUMMY-SECRET-CANARY',TURSO_AUTH_TOKEN:'DUMMY-SECRET-CANARY'};
execFileSync(process.execPath,['external/cf-build.mjs'],{env});
const root='../.cloudflare/output/v0/workers/default/';
const cfg=JSON.parse(readFileSync(new URL(root+'worker.config.json',import.meta.url),'utf8'));
assert.equal(cfg.manifest.mainModule,'worker.mjs');
assert.equal(cfg.env.TURSO_AUTH_TOKEN.type,'secret');
for(const name of Object.keys(cfg.manifest.modules)){
 assert.match(name,/^[A-Za-z0-9-]+\.mjs$/);
 const source=readFileSync(new URL(root+'bundle/'+name,import.meta.url),'utf8');
 assert(!source.includes('DUMMY-SECRET-CANARY'));
 for(const [,ref] of source.matchAll(/^import .* from ['"]([^'"]+)['"]/gm)){
  assert.match(ref,/^\.\/[A-Za-z0-9-]+\.mjs$/);
  assert(Object.hasOwn(cfg.manifest.modules,ref.slice(2)));
 }
}
const worker=(await import(new URL(root+'bundle/'+cfg.manifest.mainModule,import.meta.url))).default;
assert.equal((await worker.fetch(new Request('https://worker.test/api/bootstrap'),env)).status,401);
execFileSync(process.execPath,['scripts/build.mjs']);
const app=(await import('../dist/server/index.js')).default;
for(const url of Object.keys(assets))assert.equal((await app.fetch(new Request('https://local.test'+url),{})).status,200);
assert.equal((await app.fetch(new Request('https://local.test/.env'),{})).status,404);
assert.equal((await app.fetch(new Request('https://local.test/',{method:'POST'}),{})).status,405);
assert.equal(spawnSync(process.execPath,['external/migrate.mjs','--apply','--unexpected'],{env}).status,1);
assert.equal(spawnSync(process.execPath,['external/cf-build.mjs','--unexpected'],{env}).status,1);
console.log('PASS: builds, complete asset manifest, unsigned denial, and operator argument guard.');
