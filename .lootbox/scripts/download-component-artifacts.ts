const files = [
  [
    "espressif/esp32-s3r8",
    "esp32-s3-datasheet.pdf",
    "https://www.espressif.com/sites/default/files/documentation/esp32-s3_datasheet_en.pdf",
  ],
  [
    "espressif/esp32-s3r8",
    "esp32-s3-technical-reference-manual.pdf",
    "https://www.espressif.com/sites/default/files/documentation/esp32-s3_technical_reference_manual_en.pdf",
  ],
  [
    "espressif/esp32-u4wdh",
    "esp32-datasheet.pdf",
    "https://www.espressif.com/sites/default/files/documentation/esp32_datasheet_en.pdf",
  ],
  [
    "espressif/esp32-u4wdh",
    "esp32-technical-reference-manual.pdf",
    "https://www.espressif.com/sites/default/files/documentation/esp32_technical_reference_manual_en.pdf",
  ],
  ["texas-instruments/pcm5100a", "pcm5100a-datasheet.pdf", "https://www.ti.com/lit/gpn/PCM5100A"],
  ["texas-instruments/drv2605l", "drv2605l-datasheet.pdf", "https://www.ti.com/lit/gpn/DRV2605L"],
  ["texas-instruments/tlv62569dbvt", "tlv62569-datasheet.pdf", "https://www.ti.com/lit/gpn/TLV62569"],
  [
    "hynitron/cst816d",
    "cst816d-datasheet-v1.3.pdf",
    "https://files.waveshare.com/wiki/common/CST816D_datasheet_En_V1.3.pdf",
  ],
  ["sitronix/st77916", "st77916-spec-v1.0.pdf", "https://dl.espressif.com/AE/esp-iot-solution/ST77916_SPEC_V1.0.pdf"],
  [
    "winbond/w25q128jvpiq",
    "w25q128jv-datasheet-rev-f.pdf",
    "https://www.winbond.com/resource-files/w25q128jv%20revf%2003272018%20plus.pdf",
  ],
  ["sgmicro/sgm2036-3.3", "sgm2036-datasheet.pdf", "https://www.sg-micro.com/uploads/soft/20240814/1723624970.pdf"],
  [
    "alpha-and-omega-semiconductor/ao3400a",
    "ao3400a-datasheet.pdf",
    "https://www.aosmd.com/sites/default/files/res/datasheets/AO3400A.pdf",
  ],
  [
    "memsensing/msm261d4030h1cpm",
    "msm261d4030h1cpm-datasheet.pdf",
    "https://datasheet.lcsc.com/lcsc/1811081617_MEMSensing-MSM261D4030H1CPM_C74250.pdf",
  ],
] as const;

const failures: string[] = [];
for (const [component, name, url] of files) {
  const directory = `doc/hardware/components/${component}/artifacts`;
  const path = `${directory}/${name}`;
  await Deno.mkdir(directory, { recursive: true });
  try {
    const response = await fetch(url, {
      redirect: "follow",
      headers: { "User-Agent": "Mozilla/5.0 hardware-research/1.0" },
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    if (bytes.length < 4 || new TextDecoder().decode(bytes.slice(0, 4)) !== "%PDF") {
      throw new Error(`response is not PDF (${response.headers.get("content-type") ?? "unknown type"})`);
    }
    await Deno.writeFile(path, bytes);
    console.log(`OK ${bytes.length} ${path} <- ${response.url}`);
  } catch (error) {
    const message = `${component}/${name}: ${url}: ${error}`;
    failures.push(message);
    console.log(`FAILED ${message}`);
  }
}
await Deno.writeTextFile(
  "doc/hardware/component-download-failures.txt",
  failures.join("\n") + (failures.length ? "\n" : ""),
);
