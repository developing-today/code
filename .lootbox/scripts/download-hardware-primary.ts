const root = "doc/hardware/devices/waveshare/esp32-s3-knob-touch-lcd-1.8/artifacts";
const downloads = [
  {
    name: "ESP32-S3-Knob-Touch-LCD-1.8-schematic.zip",
    url: "https://files.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8/ESP32-S3-Knob-Touch-LCD-1.8-schematic.zip",
    sha256: "baa5ac1bf75fbbd86a8135b123ff498bd7db4a5c68184481db6b82cadbaca0e5",
    extract: "schematic",
  },
  {
    name: "ESP32-S3-Knob-Touch-LCD-1.8-Demo.zip",
    url: "https://files.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8/ESP32-S3-Knob-Touch-LCD-1.8-Demo.zip",
    sha256: "11e382444fe93470fbe463829c1e0ebad5bdb5115fd2d72f6159cd7700015030",
    extract: "demo",
  },
  {
    name: "ESP32-S3-Knob-Touch-LCD-1.8-BIN.zip",
    url: "https://files.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8/ESP32-S3-Knob-Touch-LCD-1.8-BIN.zip",
    sha256: "7d29fc1fb356059f7291eccd74bfb5c9fa7538998bc3f5ff811cd87f04c1691c",
    extract: "firmware",
  },
  {
    name: "ESP32-S3-Knob-Touch-LCD-1.8-14.jpg",
    url: "https://www.waveshare.com/w/upload/9/9d/ESP32-S3-Knob-Touch-LCD-1.8-14.jpg",
    extract: null,
  },
] as const;

async function sha256(path: string) {
  const bytes = await Deno.readFile(path);
  const hash = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(hash), (b) => b.toString(16).padStart(2, "0")).join("");
}

await Deno.mkdir(`${root}/originals`, { recursive: true });
for (const item of downloads) {
  const path = `${root}/originals/${item.name}`;
  try {
    await Deno.stat(path);
    console.log(`EXISTS ${path}`);
  } catch {
    const response = await fetch(item.url, { redirect: "follow" });
    if (!response.ok) throw new Error(`${item.url}: HTTP ${response.status}`);
    await Deno.writeFile(path, new Uint8Array(await response.arrayBuffer()));
    console.log(`DOWNLOADED ${path}`);
  }
  const actual = await sha256(path);
  const stat = await Deno.stat(path);
  console.log(`${actual} ${stat.size} ${path}`);
  if ("sha256" in item && item.sha256 !== actual) {
    throw new Error(`SHA-256 mismatch for ${item.name}: expected ${item.sha256}, got ${actual}`);
  }
  if (item.extract) {
    const destination = `${root}/${item.extract}`;
    await Deno.mkdir(destination, { recursive: true });
    const command = new Deno.Command("unzip", { args: ["-o", path, "-d", destination] });
    const result = await command.output();
    if (!result.success) throw new Error(new TextDecoder().decode(result.stderr));
    console.log(`EXTRACTED ${path} -> ${destination}`);
  }
}
