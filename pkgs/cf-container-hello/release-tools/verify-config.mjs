// Fail closed on identity, container image pins, and SQLite Durable Object
// class exports before a staged Build Output is considered.
import { readFileSync } from "node:fs";

const config = readFileSync(new URL("../wrangler.jsonc", import.meta.url), "utf8");
const source = readFileSync(new URL("../src/index.ts", import.meta.url), "utf8");
const mustContain = (value, pattern, description) => {
  if (!pattern.test(value)) throw new Error(`probe config verification failed: ${description}`);
};

mustContain(config, /"name"\s*:\s*"cf-container-hello"/, "standalone Worker identity");
mustContain(config, /"class_name"\s*:\s*"CrawlerProbe"[\s\S]*?"image"\s*:\s*"registry\.cloudflare\.com\/[^"\s]+\/lab-ops-hub-crawlercontainer:v5"/, "CrawlerProbe v5 image");
mustContain(config, /"new_sqlite_classes"\s*:\s*\[\s*"HelloContainer"\s*\]/, "HelloContainer SQLite export");
mustContain(config, /"new_sqlite_classes"\s*:\s*\[\s*"CrawlerProbe"\s*,\s*"HelloLite"\s*\]/, "CrawlerProbe and HelloLite SQLite exports");
for (const name of ["HelloContainer", "CrawlerProbe", "HelloLite"]) {
  mustContain(source, new RegExp(`export class ${name}\\s+extends Container`), `${name} source export`);
}
console.log("verified standalone identity, CrawlerProbe :v5 pin, and all SQLite container exports");
