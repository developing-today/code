// `npm test`: the unit check, then every test file in its own process.
// tests/local-server.mjs is a manual dev server, not a test.
import {spawnSync} from 'node:child_process';
import {readdirSync} from 'node:fs';
const files=['scripts/check.mjs',...readdirSync('tests').filter(f=>f.endsWith('.mjs')&&f!=='local-server.mjs').sort().map(f=>'tests/'+f)];
const failed=files.filter(f=>spawnSync(process.execPath,[f],{stdio:'inherit'}).status!==0);
if(failed.length){console.error('FAILED: '+failed.join(', '));process.exit(1)}
console.log('All '+files.length+' test files passed.');
