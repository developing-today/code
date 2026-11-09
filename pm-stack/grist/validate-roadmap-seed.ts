import seed from "./seed/roadmap.json" with { type: "json" };
import { validateRoadmapHorizon } from "../aggregator/src/roadmap-horizon.ts";

const invalid = seed.rows.flatMap((row, index) => {
  const result = validateRoadmapHorizon(row.Horizon);
  return result.valid ? [] : [`row ${index + 1} (${row.Title}): ${row.Horizon} — ${result.error}`];
});

if (invalid.length) {
  console.error(`Invalid roadmap Horizon values in seed/roadmap.json:\n${invalid.map((line) => `- ${line}`).join("\n")}`);
  process.exitCode = 1;
} else {
  console.log(`Validated ${seed.rows.length} roadmap seed Horizon values.`);
}
