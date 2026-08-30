const root = "doc/hardware/devices/waveshare/esp32-s3-knob-touch-lcd-1.8/artifacts/source-snapshots";
const pages = [
  ["waveshare-product-31623.html", "https://www.waveshare.com/esp32-s3-knob-touch-lcd-1.8.htm"],
  ["waveshare-wiki-current.html", "https://www.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8"],
  [
    "waveshare-wiki-oldid-111069.html",
    "https://www.waveshare.com/w/index.php?title=ESP32-S3-Knob-Touch-LCD-1.8&oldid=111069",
  ],
  [
    "waveshare-wiki-oldid-111069-api.json",
    "https://www.waveshare.com/w/api.php?action=query&prop=revisions&revids=111069&rvprop=ids%7Ctimestamp%7Cuser%7Ccomment&format=json",
  ],
] as const;
const repos = [
  "VolosR/Knob18Meters",
  "ihayri/ESP32-S3-1.8inch-Knob-Display-Development-Board",
  "muness/roon-knob",
  "0015/lvgl_kawaii_face",
  "knobby-mtg/knobby-mtg-life-counter",
  "juggernautwins608/Waveshare-1.8-Magic-the-Gathering-Life-Counter",
  "EmbeddedWizardGUI/ESP32-S3-Knob-Touch-LCD-1.8-EN",
  "chris023/orion-waveshare-rotary-dial",
];
await Deno.mkdir(root, { recursive: true });
for (const [name, url] of pages) {
  try {
    const response = await fetch(url, { redirect: "follow", headers: { "User-Agent": "hardware-research/1.0" } });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    await Deno.writeFile(`${root}/${name}`, bytes);
    console.log(`OK ${bytes.length} ${root}/${name}`);
  } catch (error) {
    console.log(`FAILED ${url}: ${error}`);
  }
}
const metadata = [];
for (const repo of repos) {
  const response = await fetch(`https://api.github.com/repos/${repo}`, {
    headers: { Accept: "application/vnd.github+json", "User-Agent": "hardware-research/1.0" },
  });
  if (!response.ok) {
    metadata.push({ repository: repo, error: `HTTP ${response.status}`, retrieved: "2026-08-21" });
    continue;
  }
  const data = await response.json();
  metadata.push({
    repository: repo,
    html_url: data.html_url,
    default_branch: data.default_branch,
    pushed_at: data.pushed_at,
    updated_at: data.updated_at,
    license: data.license?.spdx_id ?? null,
    archived: data.archived,
    retrieved: "2026-08-21",
  });
}
await Deno.writeTextFile(`${root}/community-repositories-2026-08-21.json`, JSON.stringify(metadata, null, 2) + "\n");
