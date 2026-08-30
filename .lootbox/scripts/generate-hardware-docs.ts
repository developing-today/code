const root = "doc/hardware";
const devicePath = `${root}/devices/waveshare/esp32-s3-knob-touch-lcd-1.8`;
const retrieved = "2026-08-21";

type Component = {
  path: string;
  title: string;
  kind: string;
  capabilities: string;
  limits: string;
  wiring: string;
  firmware: string;
  versions: string;
  caveats: string;
  sources: Array<[string, string, string]>;
};

const components: Component[] = [
  {
    path: "espressif/esp32-s3r8",
    title: "Espressif ESP32-S3R8",
    kind: "dual-core wireless MCU/SoC with in-package PSRAM",
    capabilities:
      "Two Xtensa LX7 cores up to 240 MHz, 2.4 GHz 802.11 b/g/n Wi-Fi, Bluetooth 5 LE, USB 2.0 OTG and USB Serial/JTAG, vector instructions, 512 KB SRAM and 8 MB octal PSRAM in the R8 variant.",
    limits:
      "No Classic Bluetooth. The R8 suffix identifies 8 MB in-package PSRAM, not flash; this board supplies 16 MB external flash. GPIO electrical, strapping, ADC and radio limits remain those in the datasheet.",
    wiring:
      "Primary application/display MCU. QSPI LCD: GPIO13 clock, 14 CS, 15-18 D0-D3, 21 reset, 47 backlight. Touch/haptic I2C: GPIO11 SDA, 12 SCL; touch INT 9 and reset 10. SDMMC: GPIO2 D3, 3 CMD, 4 CLK, 5 D0, 6 D1, 42 D2. Encoder: GPIO8/A and GPIO7/B. PDM MIC: GPIO45 clock and 46 data. I2S DAC: GPIO39 BCK, 40 LRCK/WS, 41 data. USB D-/D+ are GPIO19/20; inter-MCU UART is GPIO48 RX and GPIO38 TX; battery ADC is GPIO1.",
    firmware:
      "Supported by ESP-IDF and Arduino-ESP32. Use target `esp32s3`; official examples use ESP-IDF 5.1.4 and require Arduino-ESP32 >=3.2.0 for Arduino builds.",
    versions:
      "ESP32-S3 family datasheet and TRM snapshots retrieved on the date below; package marking/silicon revision is not stated in the board documents.",
    caveats:
      "Do not describe the board's 16 MB W25Q128 flash as part of ESP32-S3R8. GPIO0 is also brought into the board's unusual USB/target selection circuitry.",
    sources: [
      [
        "ESP32-S3 Series Datasheet",
        "https://www.espressif.com/sites/default/files/documentation/esp32-s3_datasheet_en.pdf",
        "artifacts/esp32-s3-datasheet.pdf",
      ],
      [
        "ESP32-S3 Technical Reference Manual",
        "https://www.espressif.com/sites/default/files/documentation/esp32-s3_technical_reference_manual_en.pdf",
        "artifacts/esp32-s3-technical-reference-manual.pdf",
      ],
      [
        "ESP-IDF ESP32-S3 Programming Guide 5.1.4",
        "https://docs.espressif.com/projects/esp-idf/en/v5.1.4/esp32s3/",
        "-",
      ],
    ],
  },
  {
    path: "espressif/esp32-u4wdh",
    title: "Espressif ESP32-U4WDH",
    kind: "dual-core wireless MCU/SiP with embedded flash",
    capabilities:
      "Dual Xtensa LX6 cores up to 240 MHz, Wi-Fi 802.11 b/g/n, Bluetooth 4.2 BR/EDR and BLE, 520 KB SRAM; U4WDH integrates 4 MB flash in the package.",
    limits:
      "Original ESP32 family: no native USB device controller and no Bluetooth 5 LE feature set. GPIO6-11 are associated with embedded flash and are not general-purpose board connections.",
    wiring:
      "Secondary MCU for Classic Bluetooth/audio and HID roles. Encoder 2 uses GPIO19/A and GPIO22/B. I2S DAC pins are GPIO25 BCK, GPIO27 LRCK/WS and GPIO26 data. Inter-MCU UART uses GPIO23 to S3 RX and GPIO18 from S3 TX. USB-UART bridge connects to U0TXD/U0RXD; GPIO0 and EN participate in automatic download.",
    firmware:
      "ESP-IDF target `esp32` and Arduino-ESP32 support it. The supplied ESP32 factory image reports ESP-IDF v5.4-727-g5cbd2a3877.",
    versions: "ESP32 family documents downloaded from Espressif; exact silicon revision and ECO are not stated.",
    caveats:
      "Product copy's aggregate '16 MB Flash' belongs to the S3 external flash; this U4WDH has its own 4 MB embedded flash. Classic Bluetooth functionality is supplied by this MCU, not ESP32-S3.",
    sources: [
      [
        "ESP32 Series Datasheet",
        "https://www.espressif.com/sites/default/files/documentation/esp32_datasheet_en.pdf",
        "artifacts/esp32-datasheet.pdf",
      ],
      [
        "ESP32 Technical Reference Manual",
        "https://www.espressif.com/sites/default/files/documentation/esp32_technical_reference_manual_en.pdf",
        "artifacts/esp32-technical-reference-manual.pdf",
      ],
    ],
  },
  {
    path: "winbond/w25q128jvpiq",
    title: "Winbond W25Q128JVPIQ",
    kind: "128 Mbit serial NOR flash",
    capabilities:
      "16 MB nonvolatile storage with Standard/Dual/Quad SPI, execute-in-place support, 4 KB sectors and JEDEC identification. `PIQ` denotes the package/order variant shown in the schematic.",
    limits:
      "Finite program/erase endurance and page/sector constraints apply. It is a 3 V device and must not be treated as the ESP32-S3R8's in-package PSRAM.",
    wiring:
      "U3 on the ESP32-S3 SPI flash interface: CS SPICS0, IO0 SPID, IO1 SPIQ, IO2 SPIWP, IO3 SPIHD, clock SPICLK; powered from VDD_SPI.",
    firmware:
      "Managed by ESP-IDF bootloader, partition-table and SPI flash APIs; normal applications do not bit-bang these pins.",
    versions: "W25Q128JV datasheet Rev F (file name as published); exact die revision/date code is unavailable.",
    caveats: "The schematic is the evidence for the full `W25Q128JVPIQ` ordering code; no BOM is supplied.",
    sources: [
      [
        "W25Q128JV Datasheet Rev F",
        "https://www.winbond.com/resource-files/w25q128jv%20revf%2003272018%20plus.pdf",
        "artifacts/w25q128jv-datasheet-rev-f.pdf",
      ],
    ],
  },
  {
    path: "sitronix/st77916",
    title: "Sitronix ST77916",
    kind: "LCD display controller",
    capabilities:
      "Controller for round 360 x 360 TFT modules with serial/QSPI command and pixel transport; specification snapshot is version 1.0.",
    limits:
      "The board exposes a 4-data-line QSPI interface rather than a framebuffer or RGB parallel bus. Controller limits depend on panel/module integration, which Waveshare does not identify.",
    wiring:
      "Board LCD connector: S3 GPIO13 QSPI clock, 14 CS, 15-18 D0-D3, 21 reset, 47 backlight; LCD_TE is shown at the connector but is not assigned to an S3 GPIO in the schematic. Panel supply is 3.3 V.",
    firmware:
      "Official example uses an `esp_lcd_sh8601`-named compatibility driver and a board-specific initialization table. Use the supplied initialization sequence unless a confirmed ST77916 driver targets this exact panel.",
    versions: "ST77916 specification V1.0. Board/module controller revision is not marked in supplied material.",
    caveats:
      "Conflict: product page says ST77916 while source symbols, driver filenames and ID constant say SH8601. This record documents the marketed controller, not proof that every unit is electrically identical.",
    sources: [
      [
        "ST77916 Specification V1.0",
        "https://dl.espressif.com/AE/esp-iot-solution/ST77916_SPEC_V1.0.pdf",
        "artifacts/st77916-spec-v1.0.pdf",
      ],
      ["Waveshare product page", "https://www.waveshare.com/esp32-s3-knob-touch-lcd-1.8.htm", "-"],
    ],
  },
  {
    path: "generic/sh8601-compatibility-driver",
    title: "SH8601 compatibility driver record",
    kind: "display-driver compatibility/conflict record",
    capabilities:
      "The official Arduino and ESP-IDF LCD examples include `esp_lcd_sh8601.c/.h`, QSPI opcodes and an initialization table used successfully for the fitted 360 x 360 display.",
    limits:
      "This is a software naming/compatibility record, not confirmation of a fitted SH8601 IC. No authoritative SH8601 datasheet was supplied or downloaded.",
    wiring: "Uses the same QSPI and control signals as the ST77916/display module: GPIO13-18, 21 and 47.",
    firmware:
      "`esp_lcd_new_panel_sh8601()` and `SH8601_PANEL_*_QSPI_CONFIG` in the official archive. The code defines ID `0x86`, but does not read and prove the controller identity.",
    versions:
      "Driver version macros are present but the example suppresses its version log; archive publication version is not stated.",
    caveats:
      "Keep the distinction between marketed ST77916 identity and SH8601-compatible source naming. Do not silently substitute one datasheet for the other.",
    sources: [
      [
        "Official demo archive",
        "https://files.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8/ESP32-S3-Knob-Touch-LCD-1.8-Demo.zip",
        "../../../devices/waveshare/esp32-s3-knob-touch-lcd-1.8/artifacts/originals/ESP32-S3-Knob-Touch-LCD-1.8-Demo.zip",
      ],
      [
        "ST77916 specification",
        "https://dl.espressif.com/AE/esp-iot-solution/ST77916_SPEC_V1.0.pdf",
        "../../sitronix/st77916/artifacts/st77916-spec-v1.0.pdf",
      ],
    ],
  },
  {
    path: "hynitron/cst816d",
    title: "Hynitron CST816D / CST816 family",
    kind: "capacitive touch controller",
    capabilities:
      "Single-point capacitive touch controller with I2C register interface, interrupt, reset, gesture and low-power modes; V1.3 document covers CST816D.",
    limits:
      "The exact package marking is not visible in supplied files. Firmware names the family `CST816`; coordinate and gesture behavior must follow the fitted firmware/register version.",
    wiring:
      "I2C address 0x15 on S3 GPIO11 SDA/GPIO12 SCL, GPIO9 interrupt and GPIO10 reset; 3.3 V logic. It shares I2C with DRV2605L.",
    firmware:
      "Official LVGL examples contain a small `cst816` driver; bundled SensorLib 0.3.1 also provides `TouchDrvCST816`. ESP-IDF I2C master and Arduino Wire can access address 0x15.",
    versions: "CST816D datasheet English V1.3; fitted silicon/firmware revision unknown.",
    caveats: "Treat CST816D as the best supported family identification, not an independently verified top marking.",
    sources: [
      [
        "CST816D Datasheet EN V1.3",
        "https://files.waveshare.com/wiki/common/CST816D_datasheet_En_V1.3.pdf",
        "artifacts/cst816d-datasheet-v1.3.pdf",
      ],
    ],
  },
  {
    path: "texas-instruments/pcm5100a",
    title: "Texas Instruments PCM5100A",
    kind: "stereo audio DAC",
    capabilities:
      "2-channel 32-bit, 384 kHz PCM DAC with integrated PLL and line-level analog outputs; accepts I2S-family serial audio without an external master clock in common configurations.",
    limits:
      "This is a DAC with line outputs, not a headphone power amplifier or speaker amplifier. Output loading and analog supply requirements from the datasheet apply.",
    wiring:
      "I2S is selected from either MCU by CH445P: S3 GPIO39 BCK, 40 LRCK/WS, 41 DIN; ESP32 GPIO25 BCK, 27 LRCK/WS, 26 DIN. OUTL/OUTR route to the 3.5 mm connector. XSMT is controlled by ESP32 GPIO32; FMT is strapped low and SCK is grounded in the schematic. Separate 3V3_DAC supply from SGM2036.",
    firmware:
      "Use ESP-IDF I2S standard TX or Arduino ESP32 I2S APIs. Official audio example emits 44.1 kHz, 16-bit data and does not require a PCM5100A control bus.",
    versions: "Current TI datasheet snapshot retrieved below; fitted lot/revision unknown.",
    caveats:
      "External amplified speakers or suitable headphones/load are required; the board contains no speaker and documentation should call this DAC line output.",
    sources: [
      ["PCM5100A Datasheet", "https://www.ti.com/lit/gpn/PCM5100A", "artifacts/pcm5100a-datasheet.pdf"],
      ["PCM5100A product page", "https://www.ti.com/product/PCM5100A", "-"],
    ],
  },
  {
    path: "texas-instruments/drv2605l",
    title: "Texas Instruments DRV2605L",
    kind: "haptic driver",
    capabilities:
      "Closed-loop/open-loop ERM and LRA haptic driver with I2C control, waveform library, auto calibration and trigger input.",
    limits:
      "Actuator electrical parameters and calibration must match the fitted motor. Maximum ratings and drive modes are not interchangeable between arbitrary ERM/LRA parts.",
    wiring:
      "U13 marked DRV2605LDGSR, 3.3 V. I2C address 0x5A shares S3 GPIO11 SDA/GPIO12 SCL with touch. EN is tied 3.3 V and IN/TRIG is tied ground in schematic; OUT+/- connect to LRA_P/LRA_N.",
    firmware:
      "Bundled SensorLib 0.3.1 `SensorDRV2605` and official `03_DRV2605_Test`; ESP-IDF I2C master or any DRV2605L register driver can be used.",
    versions:
      "Schematic specifies full `DRV2605LDGSR`; product/wiki shorthand says DRV2605. Fitted revision/date code unknown.",
    caveats: "Use DRV2605L limits and register behavior. Product shorthand must not erase the schematic's L suffix.",
    sources: [
      ["DRV2605L Datasheet", "https://www.ti.com/lit/gpn/DRV2605L", "artifacts/drv2605l-datasheet.pdf"],
      ["DRV2605L product page", "https://www.ti.com/product/DRV2605L", "-"],
    ],
  },
  {
    path: "wch/ch445p",
    title: "WCH CH445P",
    kind: "four-channel 2:1 analog switch",
    capabilities:
      "Four-pole double-throw, low-resistance analog switch used to steer digital audio signals from either MCU.",
    limits:
      "Not an audio codec and not a USB-C orientation switch. Signal voltage, bandwidth and on-resistance limits must follow the WCH datasheet; no local manufacturer PDF was located in this pass.",
    wiring:
      "U18 switches I2S BCK, DIN and LRCK/WS between ESP32-S3 and ESP32 toward PCM5100A. `I2S_SWITCH_IN` selects the source; EN# is grounded (enabled), 3V3_DAC supply.",
    firmware: "Drive the select net to choose one MCU before enabling I2S output; no serial API or address exists.",
    versions: "CH445P marking from schematic; silicon revision unknown.",
    caveats:
      "The source of `I2S_SWITCH_IN` is not clearly resolved by the five schematic PNGs, so control ownership is unresolved.",
    sources: [["WCH CH445 product search", "https://www.wch-ic.com/search?q=CH445", "-"]],
  },
  {
    path: "memsensing/msm261d4030h1cpm",
    title: "MEMSensing MSM261D4030H1CPM",
    kind: "digital PDM MEMS microphone",
    capabilities: "Digital MEMS microphone with PDM clock/data interface and channel-select input.",
    limits:
      "Exact acoustic limits should be taken only from the manufacturer/distributor datasheet. The requested distributor URL returned HTML rather than a PDF on the retrieval date.",
    wiring: "MIC1 at 3.3 V; L/R is grounded, clock to S3 GPIO45 and data to S3 GPIO46. Four ground pads are shown.",
    firmware:
      "Use ESP-IDF I2S PDM RX; official audio example configures GPIO45/46 at 44.1 kHz output sampling. No I2C control or address.",
    versions: "Full schematic ordering string `MSM261D4030H1CPM`; revision/date code unknown.",
    caveats: "No usable local datasheet was downloaded; avoid quoting unsupported sensitivity/SNR values.",
    sources: [
      [
        "Distributor datasheet endpoint (failed: HTML)",
        "https://datasheet.lcsc.com/lcsc/1811081617_MEMSensing-MSM261D4030H1CPM_C74250.pdf",
        "-",
      ],
    ],
  },
  {
    path: "texas-instruments/tlv62569dbvt",
    title: "Texas Instruments TLV62569DBVT",
    kind: "3.3 V step-down converter",
    capabilities: "Synchronous buck regulator used to derive the board 3.3 V rail efficiently from 5 V.",
    limits:
      "Input/output current, thermal and stability limits depend on layout, inductor and capacitors. The board uses an adjustable feedback network rather than relying on the part name for output voltage.",
    wiring:
      "U19 DBV package: 5 V VIN, 2.2 uH L4 from SW to 3V3, feedback divider 100 k/22.1 k producing approximately 3.3 V; EN tied to 5 V.",
    firmware: "No firmware API; always enabled when 5 V is present.",
    versions: "Schematic label `TLV62569DBVT`; fitted lot/revision unknown.",
    caveats: "This is the main 3.3 V buck, not the separate DAC LDO or unidentified charger/power-path IC.",
    sources: [
      ["TLV62569 Datasheet", "https://www.ti.com/lit/gpn/TLV62569", "artifacts/tlv62569-datasheet.pdf"],
      ["TLV62569 product page", "https://www.ti.com/product/TLV62569", "-"],
    ],
  },
  {
    path: "sgmicro/sgm2036-3.3",
    title: "SGMicro SGM2036-3.3",
    kind: "low-noise 3.3 V LDO",
    capabilities: "Low-dropout regulator providing the isolated 3V3_DAC rail from 5 V for the audio circuitry.",
    limits:
      "Current, dropout, noise and capacitor requirements require the authoritative datasheet. The attempted direct SGMicro PDF returned HTTP 404.",
    wiring:
      "U20 marked `SGM2036-3.3YN5G/TR`; IN and EN at 5 V, OUT to 3V3_DAC, BP/FB bypass capacitor 10 nF, 1 uF input/output capacitors.",
    firmware: "No firmware control; enable is tied high.",
    versions: "Fixed 3.3 V YN5G/TR ordering string from schematic; revision unknown.",
    caveats:
      "Direct PDF failed on 2026-08-21; retain the manufacturer product link and do not substitute an unverified mirror.",
    sources: [
      ["SGM2036 manufacturer product page", "https://www.sg-micro.com/product/SGM2036", "-"],
      ["SGMicro direct PDF (failed HTTP 404)", "https://www.sg-micro.com/uploads/soft/20240814/1723624970.pdf", "-"],
    ],
  },
  {
    path: "alpha-and-omega-semiconductor/ao3400a",
    title: "AOS AO3400A",
    kind: "N-channel MOSFET",
    capabilities: "Logic-level N-channel MOSFET used as a low-side LCD backlight switch/PWM element.",
    limits:
      "Gate, drain, thermal and current limits are package/layout dependent; it does not regulate LED current by itself.",
    wiring:
      "Q1 gate from S3 GPIO47 `LCD_BLK` with 10 k pulldown; drain sinks LCD LEDK through 3.9 ohm R3; LEDA is 3.3 V.",
    firmware: "Drive GPIO47 or LEDC PWM to control backlight. Official example exposes brightness values 0-255.",
    versions: "Schematic text appears `AO3400A` (font resembles A03400A); manufacturer datasheet downloaded.",
    caveats: "Backlight polarity is active high at the MCU gate. Respect boot-time default/pulldown behavior.",
    sources: [
      [
        "AO3400A Datasheet",
        "https://www.aosmd.com/sites/default/files/res/datasheets/AO3400A.pdf",
        "artifacts/ao3400a-datasheet.pdf",
      ],
    ],
  },
  {
    path: "alps-alpine/sscm110100",
    title: "Alps Alpine SSCM110100",
    kind: "directional/slide switch",
    capabilities: "Compact mechanical changeover switch; two are used beneath the dual-action knob mechanism.",
    limits:
      "The schematic only exposes A/B and common contacts; mechanical life/force/travel require the exact Alps specification. No local datasheet was downloaded.",
    wiring:
      "SW2 provides EC1_A/EC1_B and SW1 provides EC2_A/EC2_B. Pull-ups are 10 k to 3.3 V; EC1 maps to S3 GPIO8/7, EC2 maps to ESP32 GPIO19/22.",
    firmware:
      "Treat each pair as quadrature/directional inputs with debouncing; official examples use `bidi_switch_knob` and GPIO interrupts.",
    versions: "Part string `SSCM110100` appears twice in schematic; revision/date code unknown.",
    caveats:
      "Waveshare calls the assembly a dual encoder, but the schematic part is a directional switch. Do not assume a conventional detented rotary encoder contact topology beyond tested behavior.",
    sources: [
      [
        "Alps Alpine switch product search",
        "https://tech.alpsalpine.com/e/products/category/switch/sub/01/series/sscm/",
        "-",
      ],
    ],
  },
  {
    path: "generic/micro-sd-sdmmc",
    title: "microSD / SDMMC interface",
    kind: "removable storage interface",
    capabilities: "microSD/TF socket wired for native 4-bit SDMMC and FAT filesystem use.",
    limits:
      "Card capacity/speed compatibility is firmware and card dependent. No card-detect signal is connected in the supplied schematic despite a socket switch pin.",
    wiring:
      "S3 GPIO2 D3, GPIO3 CMD, GPIO4 CLK, GPIO5 D0, GPIO6 D1 and GPIO42 D2. CMD and D0-D3 have 10 k pull-ups; 3.3 V supply. Socket is labeled TF-018.",
    firmware:
      "Official Arduino `SD_MMC` and ESP-IDF `esp_vfs_fat_sdmmc_mount`; examples default to 4-bit mode. Format FAT/FatFs.",
    versions: "Socket vendor/model beyond schematic footprint `TF-018` is unknown.",
    caveats:
      "The wiki says examples can select SPI or SDMMC, but supplied pin definitions and default initialization use 4-wire SDMMC.",
    sources: [
      ["Arduino-ESP32 SD_MMC API", "https://docs.espressif.com/projects/arduino-esp32/en/latest/api/sdmmc.html", "-"],
      [
        "ESP-IDF SDMMC API 5.1.4",
        "https://docs.espressif.com/projects/esp-idf/en/v5.1.4/esp32s3/api-reference/peripherals/sdmmc_host.html",
        "-",
      ],
    ],
  },
  {
    path: "generic/lra-motor",
    title: "LRA vibration motor (unidentified)",
    kind: "linear resonant actuator",
    capabilities: "Provides haptic feedback under DRV2605L control.",
    limits:
      "Exact model, resonant frequency, rated voltage, impedance, stroke and maximum drive are absent. These must not be guessed.",
    wiring: "Two terminals LRA_P and LRA_N connect directly to DRV2605L OUT+ and OUT-. Physical mounting is internal.",
    firmware:
      "Configure DRV2605L for an LRA and run auto-calibration against the installed actuator before custom effects; official demo cycles waveform-library effects.",
    versions: "No maker, marking, BOM line or revision available.",
    caveats: "Replacement compatibility cannot be established from current documentation.",
    sources: [
      [
        "Board schematic archive",
        "https://files.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8/ESP32-S3-Knob-Touch-LCD-1.8-schematic.zip",
        "../../../devices/waveshare/esp32-s3-knob-touch-lcd-1.8/artifacts/originals/ESP32-S3-Knob-Touch-LCD-1.8-schematic.zip",
      ],
    ],
  },
  {
    path: "generic/lipo-102035",
    title: "LiPo 102035 battery (optional, maker unknown)",
    kind: "3.7 V lithium-polymer battery",
    capabilities: "Optional 102035-size rechargeable cell offered as a product variant.",
    limits:
      "Capacity, protection board, maximum charge/discharge current and cell maker are not stated in the supplied primary material. 102035 is a size designation, not a complete electrical specification.",
    wiring:
      "Connects to the board battery socket. Product material conflicts between PH1.25 and MX1.25 naming; verify polarity and pitch physically before connection.",
    firmware:
      "S3 GPIO1/ADC1 channel 0 reads `BATT_ADC` through a 10 k/10 k divider from the 5 V/system rail shown; official example computes system voltage. This is not a fuel gauge.",
    versions: "Optional included/not-included product variant; maker and revision unknown.",
    caveats:
      "Lithium safety depends on the unidentified charger/power-path and cell protection. Never infer connector compatibility from the shorthand alone.",
    sources: [["Waveshare product page", "https://www.waveshare.com/esp32-s3-knob-touch-lcd-1.8.htm", "-"]],
  },
  {
    path: "generic/usb-uart-bridge",
    title: "USB-UART bridge (unidentified)",
    kind: "USB-to-UART converter",
    capabilities: "Provides a serial programming/debug path for the ESP32-U4WDH with automatic boot/reset signaling.",
    limits:
      "Manufacturer, product ID, driver requirements and electrical limits are absent because U10 has only functional pin names in the schematic.",
    wiring:
      "U10 UD+/UD- connect to the USB selection network; TXD/RXD connect to ESP32 U0RXD/U0TXD, DTR#/RTS# drive ESP32 GPIO0/EN auto-download, 3.3 V supply.",
    firmware:
      "Host exposes a serial port when the correct USB-C orientation selects this path. Use esptool target `esp32`.",
    versions: "No top marking or BOM; identity unresolved.",
    caveats: "Do not claim CP210x, CH34x or another bridge based on package resemblance.",
    sources: [
      [
        "Board schematic archive",
        "https://files.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8/ESP32-S3-Knob-Touch-LCD-1.8-schematic.zip",
        "../../../devices/waveshare/esp32-s3-knob-touch-lcd-1.8/artifacts/originals/ESP32-S3-Knob-Touch-LCD-1.8-schematic.zip",
      ],
    ],
  },
  {
    path: "generic/charger-power-path",
    title: "Battery charger / power-path (unidentified)",
    kind: "battery charging and power management function",
    capabilities: "Product claims onboard battery charging management and supports an optional 3.7 V cell.",
    limits:
      "IC identity, charge voltage/current, termination, power-path behavior, battery protection and status indications are not established by the five supplied schematic PNGs.",
    wiring:
      "Battery socket and USB 5 V/system rails are present, but a complete identifiable charger block is not available in the supplied schematic pages.",
    firmware: "No confirmed control/status API or pins.",
    versions: "Unknown maker/model/revision.",
    caveats:
      "Do not select a replacement cell or make charging-safety claims without inspecting the physical IC and complete circuitry.",
    sources: [["Waveshare wiki", "https://www.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8", "-"]],
  },
  {
    path: "generic/lcd-panel-module",
    title: "1.8-inch 360 x 360 LCD panel module (unidentified)",
    kind: "round capacitive-touch TFT module",
    capabilities:
      "1.8-inch round 360 x 360 color display with capacitive touch and controllable backlight, integrated into the knob top.",
    limits:
      "Panel maker, optical specifications, module ordering code, viewing technology and full connector specification are absent. ST77916 is marketed but source names SH8601.",
    wiring:
      "28-pin schematic module symbol: QSPI, reset, TE, touch I2C/reset/interrupt, LEDA/LEDK, 3.3 V and ground. See ST77916, SH8601 compatibility, CST816D and AO3400A records.",
    firmware: "Official LVGL 8.4.0 example uses esp_lcd plus board-specific QSPI initialization and CST816 touch code.",
    versions: "No module label/revision. Controller conflict unresolved.",
    caveats:
      "Do not buy a nominally similar 1.8-inch round panel as a drop-in replacement without matching pinout, controller, initialization and mechanics.",
    sources: [
      ["Waveshare product page", "https://www.waveshare.com/esp32-s3-knob-touch-lcd-1.8.htm", "-"],
      [
        "Official demo archive",
        "https://files.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8/ESP32-S3-Knob-Touch-LCD-1.8-Demo.zip",
        "../../../devices/waveshare/esp32-s3-knob-touch-lcd-1.8/artifacts/originals/ESP32-S3-Knob-Touch-LCD-1.8-Demo.zip",
      ],
    ],
  },
  {
    path: "generic/ceramic-antenna",
    title: "2.4 GHz ceramic antennas (unidentified)",
    kind: "onboard RF antennas",
    capabilities: "Separate ceramic antennas serve the ESP32-S3 and ESP32 2.4 GHz radios.",
    limits:
      "Maker, antenna model, gain, efficiency, keep-out and certification details are not provided. Enclosure and nearby conductors affect performance.",
    wiring:
      "ANT1 follows the ESP32-S3 LNA input through a 2 nH/2.2 pF matching network; ANT2 similarly serves ESP32. Schematic symbol text is `CA-C03` but this is not enough to establish a manufacturer ordering code.",
    firmware: "No API; Wi-Fi/Bluetooth stacks use the internal RF paths.",
    versions: "Two fitted antennas; model/revision unresolved.",
    caveats: "Do not attach an external antenna or alter matching components without RF validation.",
    sources: [
      [
        "Board schematic archive",
        "https://files.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8/ESP32-S3-Knob-Touch-LCD-1.8-schematic.zip",
        "../../../devices/waveshare/esp32-s3-knob-touch-lcd-1.8/artifacts/originals/ESP32-S3-Knob-Touch-LCD-1.8-schematic.zip",
      ],
    ],
  },
  {
    path: "generic/usb-c-interface",
    title: "USB-C target-selection interface",
    kind: "USB-C power/data interface",
    capabilities: "Supplies 5 V and routes USB data to one of two MCU programming paths depending on plug orientation.",
    limits:
      "The exact orientation-switch implementation and USB-C controller are not identified. This unusual behavior is explicitly documented by Waveshare and is not normal USB-C expectation.",
    wiring:
      "One orientation reaches ESP32-S3 native USB D-/D+ on GPIO19/20; the other reaches the unidentified ESP32 USB-UART bridge. Disconnect, rotate the Type-C plug 180 degrees and reconnect to change target.",
    firmware:
      "Use `esptool chip_id` to identify the selected target. S3 can use native USB/USB CDC; ESP32 path is UART bridge.",
    versions: "Connector/switch maker and USB descriptors unknown.",
    caveats:
      "Cable orientation is operationally significant. Do not assume failed enumeration means hardware failure until the plug has been flipped.",
    sources: [["Waveshare FAQ", "https://www.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8#FAQ", "-"]],
  },
  {
    path: "generic/3.5mm-audio-output",
    title: "3.5 mm stereo line output",
    kind: "analog audio interface",
    capabilities: "Exposes PCM5100A left/right DAC outputs for external audio playback.",
    limits:
      "No onboard speaker or power amplifier. The schematic shows direct DAC OUTL/OUTR routing; load suitability is constrained by PCM5100A output specifications.",
    wiring:
      "OUTL and OUTR from PCM5100A to the 3.5 mm connector with ground. Product calls it headphone jack, while electrically it is best documented as DAC line output.",
    firmware: "Select an MCU through CH445P, configure I2S, and unmute PCM5100A via XSMT where applicable.",
    versions: "Jack maker/model and contact-detect behavior unknown.",
    caveats:
      "Use powered speakers or an appropriate external amplifier for predictable volume; headphones may work but are not evidence of a dedicated headphone amplifier.",
    sources: [
      [
        "PCM5100A Datasheet",
        "https://www.ti.com/lit/gpn/PCM5100A",
        "../../texas-instruments/pcm5100a/artifacts/pcm5100a-datasheet.pdf",
      ],
    ],
  },
  {
    path: "generic/ph1.27-expansion-connectors",
    title: "PH1.27 10-pin expansion connectors",
    kind: "board expansion interfaces",
    capabilities: "Two 10-pin, 1.27 mm-pitch SMD headers are advertised for expansion and access to board signals.",
    limits:
      "The supplied five schematic PNGs do not provide a complete connector pin-number-to-net table, and product imagery does not justify reconstructing one.",
    wiring:
      "Two physical 10-pin headers are shown in product resources. Exact pinout, voltage tolerance and current capacity remain unresolved pending a complete schematic/BOM or continuity measurement.",
    firmware: "Depends on the signals actually exposed; no generic API.",
    versions: "Connector maker/order code unknown; Waveshare calls them PH1.27 10P SMD headers.",
    caveats:
      "Do not connect peripherals based on inferred geometry. Verify every pin with continuity and voltage measurements first.",
    sources: [
      [
        "Waveshare wiki onboard resources",
        "https://www.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8#Onboard_Resources",
        "-",
      ],
    ],
  },
];

function sourceList(sources: Component["sources"]) {
  return sources.map(([title, url, local]) => `| ${title} | ${url} | ${retrieved} | ${local} |`).join("\n");
}

for (const component of components) {
  const dir = `${root}/components/${component.path}`;
  await Deno.mkdir(dir, { recursive: true });
  const readme = `# ${component.title}\n\n- **Category:** ${component.kind}\n- **Research status:** verified against available board schematic/code and linked primary material\n- **Retrieved:** ${retrieved}\n\n## Capabilities\n\n${component.capabilities}\n\n## Limits\n\n${component.limits}\n\n## Board wiring\n\n${component.wiring}\n\n## Firmware and APIs\n\n${component.firmware}\n\n## Versions and revisions\n\n${component.versions}\n\n## Caveats\n\n${component.caveats}\n\n## Used By\n\n- [Waveshare ESP32-S3-Knob-Touch-LCD-1.8](../../../devices/waveshare/esp32-s3-knob-touch-lcd-1.8/README.md)\n\n## Authoritative sources\n\n| Title | URL | Retrieved | Local artifact |\n|---|---|---:|---|\n${sourceList(component.sources)}\n`;
  await Deno.writeTextFile(`${dir}/README.md`, readme);
}

const c = (path: string, label: string) => `[${label}](../../../components/${path}/README.md)`;
const EN = (strings: TemplateStringsArray, ...values: unknown[]) => strings.reduce((text, part, index) => text + part + (values[index] ?? ""), "");
let deviceReadme =
  `# Waveshare ESP32-S3-Knob-Touch-LCD-1.8\n\n> Product ID **31623**. Research retrieved **${retrieved}**. Canonical path: \`doc/hardware/devices/waveshare/esp32-s3-knob-touch-lcd-1.8/\`; also visible through \`docs/hardware/\`.\n\n## Identity and variants\n\nWaveshare's round CNC-metal knob/display development device combines two independent wireless MCUs, a 1.8-inch 360 x 360 capacitive-touch LCD, dual directional knob inputs, removable storage, microphone, stereo DAC output and haptics. Product options are black or blue and with or without an optional 3.7 V 102035 LiPo. The wiki's -
  EN suffix describes the referenced product variant/documentation, not a demonstrated PCB hardware revision. No PCB revision, BOM or serial-number scheme is published.\n\n## Key specifications\n\n| Area | Specification | Evidence/caveat |\n|---|---|---|\n| Main MCU | ${c("espressif/esp32-s3r8", "ESP32-S3R8")}, dual LX7 up to 240 MHz, Wi-Fi, BLE 5, 8 MB PSRAM | Schematic and product page |\n| Secondary MCU | ${c("espressif/esp32-u4wdh", "ESP32-U4WDH")}, dual LX6 up to 240 MHz, Wi-Fi, Bluetooth Classic + BLE, integrated 4 MB flash | Schematic; provides Classic Bluetooth |\n| Main storage | ${c("winbond/w25q128jvpiq", "W25Q128JVPIQ")} 128 Mbit / 16 MB external flash | S3 flash; separate from U4WDH's 4 MB |\n| Display | ${c("generic/lcd-panel-module", "1.8-inch round 360 x 360 panel")}, marketed ${c("sitronix/st77916", "ST77916")}; code uses ${c("generic/sh8601-compatibility-driver", "SH8601 compatibility driver")} | Controller naming conflict unresolved |\n| Touch | ${c("hynitron/cst816d", "CST816D/CST816 family")}, I2C address 0x15, interrupt/reset | Single-point controller family |\n| Audio input | ${c("memsensing/msm261d4030h1cpm", "MSM261D4030H1CPM PDM microphone")} | S3 PDM input |\n| Audio output | ${c("texas-instruments/pcm5100a", "PCM5100A stereo DAC")} through ${c("wch/ch445p", "CH445P")} to ${c("generic/3.5mm-audio-output", "3.5 mm line output")} | No onboard speaker or amplifier |\n| Haptic | ${c("texas-instruments/drv2605l", "DRV2605L")} at I2C 0x5A and ${c("generic/lra-motor", "unknown LRA")} | Product shorthand omits L suffix |\n| Controls | Two ${c("alps-alpine/sscm110100", "SSCM110100 directional switches")} used as dual knob inputs; power and S3 BOOT buttons | Not a conventional encoder part in schematic |\n| Storage | ${c("generic/micro-sd-sdmmc", "microSD/TF, 4-bit SDMMC")} | FAT/FatFs examples |\n| Power | 5 V USB input; ${c("texas-instruments/tlv62569dbvt", "TLV62569DBVT")} main 3.3 V buck; ${c("sgmicro/sgm2036-3.3", "SGM2036-3.3")} DAC LDO; ${c("generic/charger-power-path", "charger/power-path unknown")} | Optional ${c("generic/lipo-102035", "3.7 V 102035 LiPo")} |\n| RF | Two ${c("generic/ceramic-antenna", "unidentified ceramic antennas")} | One per MCU |\n| USB | ${c("generic/usb-c-interface", "USB-C orientation-select interface")} and ${c("generic/usb-uart-bridge", "unknown USB-UART bridge")} | Plug orientation selects MCU |\n| Expansion | Two ${c("generic/ph1.27-expansion-connectors", "PH1.27 10-pin headers")} | Exact pinout absent |\n| Dimensions | 66.00 mm diameter x 22.00 mm height | Official dimension image |\n| Enclosure | CNC-machined metal, black or blue | Product page |\n\nOther fitted functional circuitry includes the ${c("alpha-and-omega-semiconductor/ao3400a", "AO3400A backlight MOSFET")}. Passive matching, filtering, pull-up and decoupling parts are intentionally not separate records.\n\n## Architecture\n\nThe ESP32-S3 owns the LCD, touch, haptics, SDMMC, PDM microphone and primary knob. The ESP32-U4WDH supplies Classic Bluetooth and owns the second knob. Both MCUs can feed the PCM5100A I2S DAC through CH445P, and communicate over a cross-connected UART. USB-C orientation physically chooses the S3 native USB path or the ESP32 USB-UART path. This architecture allows S3 GUI/storage/voice processing and ESP32 Classic Bluetooth audio/HID roles, but requires deliberate arbitration of shared audio output.\n\n## Documentation map\n\n- [Exact pinouts, buses and addressing](pinouts-and-buses.md)\n- [Development setup and official examples](development.md)\n- [Factory firmware and restore procedure](factory-firmware.md)\n- [Conflicts, limitations and unresolved gaps](gaps-and-conflicts.md)\n- [Complete source table](sources.md)\n- [Artifact manifest](../../../artifact-manifest.md)\n- [Community links and snapshot metadata](community.md)\n- [All hardware components](../../../components/README.md)\n\n## Artifact layout\n\n- \`artifacts/originals/\`: byte-preserved ZIPs and official dimension image\n- \`artifacts/schematic/\`: five official schematic PNGs extracted from the ZIP\n- \`artifacts/demo/\`: complete official Arduino/ESP-IDF source archive, including bundled licenses/notices\n- \`artifacts/firmware/\`: both extracted factory images\n- \`artifacts/source-snapshots/\`: product/wiki HTML, revision API metadata and community repository metadata\n\nNo unlicensed community source is copied locally.\n`;
await Deno.writeTextFile(`${devicePath}/README.md`, deviceReadme);

await Deno.writeTextFile(
  `${devicePath}/pinouts-and-buses.md`,
  `# Pinouts, buses and addressing\n\nRetrieved: ${retrieved}. Pin mappings below are cross-checked between the five schematic PNGs and official examples. "NC/unresolved" is intentional where evidence is incomplete.\n\n## ESP32-S3 GPIO map\n\n| GPIO | Net/function | Bus/notes |\n|---:|---|---|\n| 0 | CHIP_PU/USB selection-related net in connector block | Strapping context; do not assume freely available |\n| 1 | BATT_ADC | ADC1 channel 0; 10 k/10 k divider shown from 5 V/system rail |\n| 2 | SDMMC D3 | 4-bit SD |\n| 3 | SDMMC CMD | 10 k pull-up |\n| 4 | SDMMC CLK | |\n| 5 | SDMMC D0 | 10 k pull-up |\n| 6 | SDMMC D1 | 10 k pull-up |\n| 7 | EC1_B | primary knob input, 10 k pull-up |\n| 8 | EC1_A | primary knob input, 10 k pull-up |\n| 9 | TP_INT | CST816 touch interrupt |\n| 10 | TP_RST | CST816 touch reset |\n| 11 | TP_SDA / HAPTIC_SDA | shared I2C SDA |\n| 12 | TP_SCL / HAPTIC_SCL | shared I2C SCL |\n| 13 | LCD_QSPI_SCL | SPI2/QSPI clock |\n| 14 | LCD_QSPI_CS | display chip select |\n| 15 | LCD_QSPI_D0 | display data 0 |\n| 16 | LCD_QSPI_D1 | display data 1 |\n| 17 | LCD_QSPI_D2 | display data 2 |\n| 18 | LCD_QSPI_D3 | display data 3 |\n| 19 | USB_DN | native USB D- |\n| 20 | USB_DP | native USB D+ |\n| 21 | LCD_RST | display reset |\n| 38 | ESP32S3_TX | to ESP32 GPIO18 |\n| 39 | S3_I2S_DAC_BCK | selected through CH445P |\n| 40 | S3_I2S_DAC_LRCK/WS | selected through CH445P |\n| 41 | S3_I2S_DAC_DIN | selected through CH445P |\n| 42 | SDMMC D2 | 10 k pull-up |\n| 45 | PDM_MIC_SCK | microphone clock |\n| 46 | PDM_MIC_DATA | microphone data |\n| 47 | LCD_BLK | AO3400A gate; PWM backlight |\n| 48 | ESP32S3_RX | from ESP32 GPIO23 |\n\nUnlisted S3 pins are not claimed as available. LCD_TE exists at the panel symbol but its MCU connection is not established.\n\n## ESP32-U4WDH GPIO map\n\n| GPIO | Net/function | Notes |\n|---:|---|---|\n| 0 | ESP32_IO0 | auto-download/boot via USB-UART bridge |\n| 18 | ESP32S3_TX receive path | S3 GPIO38 -> ESP32 GPIO18 |\n| 19 | EC2_A | secondary knob |\n| 22 | EC2_B | secondary knob |\n| 23 | ESP32S3_RX transmit path | ESP32 GPIO23 -> S3 GPIO48 |\n| 25 | ESP32_I2S_DAC_BCK | CH445P selectable |\n| 26 | ESP32_I2S_DAC_DIN | CH445P selectable |\n| 27 | ESP32_I2S_DAC_LRCK/WS | CH445P selectable |\n| 32 | XSMT | PCM5100A soft mute/control |\n| U0TXD/U0RXD | USB-UART serial | unidentified bridge |\n\n## Bus inventory\n\n| Bus | Controller | Signals | Devices/address |\n|---|---|---|---|\n| I2C0 | ESP32-S3 | GPIO11 SDA, GPIO12 SCL, 3.3 V pull-ups | CST816D 0x15; DRV2605L 0x5A |\n| Display QSPI/SPI2 | ESP32-S3 | GPIO13-18 + reset 21 | marketed ST77916; software called SH8601 |\n| SDMMC 4-bit | ESP32-S3 | GPIO2-6,42 | microSD socket; no address |\n| I2S TX | either MCU through CH445P | S3 39/40/41 or ESP32 25/27/26 | PCM5100A; no address |\n| PDM RX | ESP32-S3 | GPIO45/46 | MSM261D4030H1CPM; no address |\n| UART | both MCUs | S3 TX38 -> ESP32 RX18; ESP32 TX23 -> S3 RX48 | board-internal |\n| USB | orientation selected | S3 GPIO19/20 or bridge to ESP32 UART0 | orientation matters |\n| SPI flash | ESP32-S3 dedicated | SPICS0/CLK/IO0-3 | W25Q128JVPIQ |\n\n## Connector records\n\nThe 3.5 mm output, battery socket, USB-C and PH1.27 headers are documented in their linked component/interface records. Exact PH1.27 header pin numbering is not present in the supplied five-page schematic and is therefore not fabricated. Battery connector descriptions conflict: product imagery/wiki uses MX1.25 while other product wording uses PH1.25; inspect pitch, keying and polarity before use.\n`,
);

await Deno.writeTextFile(
  `${devicePath}/development.md`,
  `# Development and examples\n\nRetrieved: ${retrieved}.\n\n## Arduino\n\n1. Install current Arduino IDE and Espressif's \`esp32\` board package **>= 3.2.0**.\n2. Open a project under \`artifacts/demo/ESP32-S3-Knob-Touch-LCD-1.8-Demo/Arduino/examples/\`.\n3. Select **ESP32S3 Dev Module**, the selected USB serial port, and enable USB CDC if using the S3 native USB path.\n4. Install bundled \`lvgl\` **8.4.0** offline for the LVGL demo and SensorLib **0.3.1** for DRV2605. Preserve the bundled license files.\n5. If no port appears, disconnect and rotate the USB-C plug 180 degrees; orientation selects the target.\n\n## ESP-IDF\n\nOfficial setup guidance references **ESP-IDF 5.1.4**. Install that release and the VS Code Espressif extension or use the command line. Open one project directory directly under \`artifacts/demo/.../ESP-IDF/\`, set target \`esp32s3\`, select the port, then run \`idf.py build flash monitor\`. The factory S3 image also identifies an IDF 5.1.4-derived build; the separate ESP32 factory image uses a later 5.4 development revision.\n\n## Official examples\n\n| Example | Purpose | Key interfaces |\n|---|---|---|\n| 01_ADC_Test | Read system/battery-divider voltage | S3 ADC1 channel 0 / GPIO1 |\n| 02_SD_Card | Mount/read/write FAT card | 4-bit SDMMC GPIO2-6,42 |\n| 03_DRV2605_Test | Cycle haptic waveform effects | I2C GPIO11/12, address 0x5A, SensorLib |\n| 04_Encoder_Test | Count knob direction/events | S3 GPIO8/7 |\n| 05_WIFI_AP | Run Wi-Fi access point | ESP32-S3 radio |\n| 06_WIFI_STA | Join Wi-Fi network | ESP32-S3 radio |\n| 07_Audio_Test | PDM microphone to I2S DAC loop/playback | GPIO45/46 PDM; GPIO39/40/41 I2S; external output device |\n| 08_LVGL_Test | Display/touch GUI and optional backlight test | QSPI LCD, CST816 I2C, LVGL 8.4.0 |\n\nBoth Arduino and ESP-IDF trees are retained completely. The archive includes much more bundled library source than the eight board examples; use only what the project requires. The only license file found by filename search is SensorLib's LICENSE, while bundled LVGL carries its upstream files/metadata; absence of a top-level Waveshare license means redistribution rights for original demo files should not be generalized beyond preserving this official archive.\n\n## Display driver note\n\nThe LVGL example declares 360 x 360, RGB565/16-bit, SPI2 QSPI and a driver named \`esp_lcd_sh8601\`. Product identity says ST77916. Keep the official command table and timings together with this exact panel until the controller discrepancy is resolved.\n\n## Useful source paths\n\n- Arduino pin definitions: \`Arduino/examples/08_LVGL_Test/lcd_config.h\`, \`07_Audio_Test/user_config.h\`, \`02_SD_Card/sd_card_bsp.cpp\`\n- Display driver: \`Arduino/examples/08_LVGL_Test/esp_lcd_sh8601.c\` and equivalent ESP-IDF component\n- Touch driver: \`Arduino/examples/08_LVGL_Test/cst816.cpp\`\n- Haptics: \`03_DRV2605_Test\` and bundled SensorLib\n- ESP-IDF examples: eight independent projects under \`ESP-IDF/\`\n`,
);

await Deno.writeTextFile(
  `${devicePath}/factory-firmware.md`,
  `# Factory firmware and flash restore\n\nRetrieved: ${retrieved}. The official BIN archive contains two complete monolithic images. Verify hashes before writing.\n\n| Target | File | Build metadata | Size | SHA-256 | Flash offset |\n|---|---|---|---:|---|---:|\n| ESP32-U4WDH | \`ESP32-KNOB_ESP32_0.bin\` | built 2025-04-18; IDF v5.4-727-g5cbd2a3877 | 1,130,672 B | \`0c1c21b9822d4c2d80d58534b33eb0083880de4ed7354a38b4c78ba51757349d\` | 0x0 |\n| ESP32-S3R8 | \`WX-ESP32S3-KNOB_V1.2.bin\` | V1.2; built 2025-02-28; IDF v5.1.4-972-g632e0c2a9f-dirty; Arduino 3.0.7 | 2,138,224 B | \`f7c1cc18b687559f3bd69e5c9ab526bc61c2b2d9c502f38367f7f2bfe4ff8e87\` | 0x0 |\n\nLocal directory: \`artifacts/firmware/ESP32-S3-Knob-Touch-LCD-1.8-BIN/\`. The outer ZIP hash is in [artifact manifest](../../../artifact-manifest.md).\n\n## Restore procedure\n\n1. Install Python and Espressif esptool. Connect USB-C and run \`esptool --port PORT --baud 115200 chip_id\`.\n2. Confirm the reported target. If it is the wrong MCU or no port appears, disconnect, rotate the Type-C plug 180 degrees, and reconnect.\n3. Flash only the matching image at offset 0x0.\n\n~~~sh\nesptool --chip esp32 --port PORT --baud 921600 write_flash -z 0x0 ESP32-KNOB_ESP32_0.bin\nesptool --chip esp32s3 --port PORT --baud 921600 write_flash -z 0x0 WX-ESP32S3-KNOB_V1.2.bin\n~~~\n\n4. Rotate the plug as needed and flash the other MCU. Power-cycle/toggle the power switch after both succeed. Reduce baud to 115200 if the connection is unreliable.\n\n## Safety\n\nThese commands overwrite each target's bootloader, partition table and application because each file is a merged image beginning at 0x0. Back up custom flash first. Never flash the ESP32 image to ESP32-S3 or vice versa; check \`chip_id\`, filename and hash each time.\n`,
);

await Deno.writeTextFile(
  `${devicePath}/gaps-and-conflicts.md`,
  `# Conflicts and unresolved gaps\n\nRetrieved: ${retrieved}. These are evidence boundaries, not assumptions to fill.\n\n| Topic | Evidence | Current conclusion |\n|---|---|---|\n| Display controller | Product says ST77916; official code/file/API names SH8601 and defines ID 0x86 | Preserve both records; fitted controller identity/compatibility is unresolved |\n| Battery connector | Described as PH1.25 in some material and MX1.25 on wiki imagery | Measure pitch and verify polarity/keying physically |\n| Haptic IC | Product/wiki shorthand DRV2605; schematic says DRV2605LDGSR | Document fitted part as DRV2605L |\n| USB-UART bridge | Functional symbol only, no part number | Unknown; do not guess CP210x/CH34x |\n| Charger/power path | Product claims charging; identifiable complete charger block absent | Charge IC and safety behavior unknown |\n| Audio wording | Called headphone jack; schematic has PCM5100A DAC but no headphone/speaker amp | Document as 3.5 mm stereo DAC line output; no onboard speaker |\n| LCD module | No maker/module number/optical spec | Replacement not determined |\n| LRA | No maker/model/electrical data | Tune/auto-calibrate fitted actuator; replacement unresolved |\n| Antennas | CA-C03 symbol only | Exact antenna identity/gain unknown |\n| Expansion headers | Advertised PH1.27 10P, but no complete connector pin table in five PNGs | Do not publish inferred pinout |\n| Schematics | Archive contains only five raster PNGs | No source schematic, BOM, Gerbers, PCB layout or mechanical CAD |\n| USB-C | FAQ says plug orientation selects MCU | Unusual but confirmed behavior; exact switching IC/topology unresolved |\n| Product revisions | No PCB/BOM revision identifier | Findings apply to published files retrieved on this date |\n\n## Failed retrievals\n\n- Immutable rendered wiki URL with \`oldid=111069\`: HTTP 404 on ${retrieved}. MediaWiki API metadata was retained and confirms revision date 2026-08-07.\n- SGMicro direct SGM2036 PDF: HTTP 404. Manufacturer product/direct links retained.\n- MSM261D4030H1CPM distributor PDF endpoint: returned HTML rather than PDF. Link retained; no substitute specifications invented.\n\n## Needed to close gaps\n\nA high-resolution board teardown with IC top markings, continuity mapping of both PH1.27 headers and battery connector, USB descriptor capture in both orientations, LCD read-ID trace, and complete Waveshare design package/BOM would resolve most open items.\n`,
);

const sources = `# Source manifest\n\nEvery source was retrieved or checked on **${retrieved}**. Local paths are relative to this device folder unless noted. A dash means link-only.\n\n| Title | URL | Publisher | Retrieved | Version/date | Local artifact | Notes |\n|---|---|---|---:|---|---|---|\n| ESP32-S3 1.8inch Knob Display product page, ID 31623 | https://www.waveshare.com/esp32-s3-knob-touch-lcd-1.8.htm | Waveshare | ${retrieved} | live page | \`artifacts/source-snapshots/waveshare-product-31623.html\` | Identity, options, specs, dimensions |\n| ESP32-S3-Knob-Touch-LCD-1.8 Wiki | https://www.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8 | Waveshare | ${retrieved} | live page | \`artifacts/source-snapshots/waveshare-wiki-current.html\` | Setup, examples, FAQ, resources |\n| Immutable wiki revision 111069 | https://www.waveshare.com/w/index.php?title=ESP32-S3-Knob-Touch-LCD-1.8&oldid=111069 | Waveshare | ${retrieved} | 2026-08-07T13:19:04Z | API metadata: \`artifacts/source-snapshots/waveshare-wiki-oldid-111069-api.json\` | Rendered URL returned HTTP 404; API confirms revision |\n| Official schematic ZIP | https://files.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8/ESP32-S3-Knob-Touch-LCD-1.8-schematic.zip | Waveshare | ${retrieved} | undated | \`artifacts/originals/ESP32-S3-Knob-Touch-LCD-1.8-schematic.zip\` | SHA-256 baa5ac...; five extracted PNGs |\n| Official demo ZIP | https://files.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8/ESP32-S3-Knob-Touch-LCD-1.8-Demo.zip | Waveshare | ${retrieved} | LVGL 8.4.0; SensorLib 0.3.1 | \`artifacts/originals/ESP32-S3-Knob-Touch-LCD-1.8-Demo.zip\` | Complete Arduino + ESP-IDF source, licenses retained |\n| Official factory BIN ZIP | https://files.waveshare.com/wiki/ESP32-S3-Knob-Touch-LCD-1.8/ESP32-S3-Knob-Touch-LCD-1.8-BIN.zip | Waveshare | ${retrieved} | S3 V1.2 / builds 2025-02-28 and 2025-04-18 | \`artifacts/originals/ESP32-S3-Knob-Touch-LCD-1.8-BIN.zip\` | Two merged images |\n| Official dimension image | https://www.waveshare.com/w/upload/9/9d/ESP32-S3-Knob-Touch-LCD-1.8-14.jpg | Waveshare | ${retrieved} | undated | \`artifacts/originals/ESP32-S3-Knob-Touch-LCD-1.8-14.jpg\` | 66 x 22 mm |\n| ESP32-S3 datasheet | https://www.espressif.com/sites/default/files/documentation/esp32-s3_datasheet_en.pdf | Espressif | ${retrieved} | live manufacturer document | \`../../../components/espressif/esp32-s3r8/artifacts/esp32-s3-datasheet.pdf\` | Manufacturer original redirects to documentation host |\n| ESP32-S3 TRM | https://www.espressif.com/sites/default/files/documentation/esp32-s3_technical_reference_manual_en.pdf | Espressif | ${retrieved} | live manufacturer document | \`../../../components/espressif/esp32-s3r8/artifacts/esp32-s3-technical-reference-manual.pdf\` | Manufacturer original |\n| ESP32 datasheet | https://www.espressif.com/sites/default/files/documentation/esp32_datasheet_en.pdf | Espressif | ${retrieved} | live manufacturer document | \`../../../components/espressif/esp32-u4wdh/artifacts/esp32-datasheet.pdf\` | Manufacturer original |\n| ESP32 TRM | https://www.espressif.com/sites/default/files/documentation/esp32_technical_reference_manual_en.pdf | Espressif | ${retrieved} | live manufacturer document | \`../../../components/espressif/esp32-u4wdh/artifacts/esp32-technical-reference-manual.pdf\` | Manufacturer original |\n| ST77916 Specification | https://dl.espressif.com/AE/esp-iot-solution/ST77916_SPEC_V1.0.pdf | Sitronix via Espressif | ${retrieved} | V1.0 | \`../../../components/sitronix/st77916/artifacts/st77916-spec-v1.0.pdf\` | Controller conflict noted |\n| CST816D Datasheet | https://files.waveshare.com/wiki/common/CST816D_datasheet_En_V1.3.pdf | Hynitron via Waveshare | ${retrieved} | English V1.3 | \`../../../components/hynitron/cst816d/artifacts/cst816d-datasheet-v1.3.pdf\` | Official board resource mirror |\n| PCM5100A Datasheet | https://www.ti.com/lit/gpn/PCM5100A | Texas Instruments | ${retrieved} | live datasheet | \`../../../components/texas-instruments/pcm5100a/artifacts/pcm5100a-datasheet.pdf\` | Manufacturer original |\n| DRV2605L Datasheet | https://www.ti.com/lit/gpn/DRV2605L | Texas Instruments | ${retrieved} | live datasheet | \`../../../components/texas-instruments/drv2605l/artifacts/drv2605l-datasheet.pdf\` | Manufacturer original |\n| TLV62569 Datasheet | https://www.ti.com/lit/gpn/TLV62569 | Texas Instruments | ${retrieved} | live datasheet | \`../../../components/texas-instruments/tlv62569dbvt/artifacts/tlv62569-datasheet.pdf\` | Manufacturer original |\n| W25Q128JV Datasheet | https://www.winbond.com/resource-files/w25q128jv%20revf%2003272018%20plus.pdf | Winbond | ${retrieved} | Rev F, file dated 2018-03-27 | \`../../../components/winbond/w25q128jvpiq/artifacts/w25q128jv-datasheet-rev-f.pdf\` | Manufacturer original |\n| AO3400A Datasheet | https://www.aosmd.com/sites/default/files/res/datasheets/AO3400A.pdf | Alpha and Omega Semiconductor | ${retrieved} | live datasheet | \`../../../components/alpha-and-omega-semiconductor/ao3400a/artifacts/ao3400a-datasheet.pdf\` | Manufacturer original |\n| SGM2036 product page | https://www.sg-micro.com/product/SGM2036 | SGMicro | ${retrieved} | live page | - | Direct PDF URL returned HTTP 404 |\n| SGM2036 direct PDF attempt | https://www.sg-micro.com/uploads/soft/20240814/1723624970.pdf | SGMicro | ${retrieved} | URL path dated 2024-08-14 | - | Failed HTTP 404 |\n| MSM261D4030H1CPM distributor PDF attempt | https://datasheet.lcsc.com/lcsc/1811081617_MEMSensing-MSM261D4030H1CPM_C74250.pdf | LCSC / MEMSensing | ${retrieved} | URL path dated 2018-11-08 | - | Returned HTML, not retained as PDF |\n| Arduino-ESP32 installation/API docs | https://docs.espressif.com/projects/arduino-esp32/en/latest/installing.html | Espressif | ${retrieved} | required >=3.2.0 by Waveshare | - | Environment reference |\n| ESP-IDF ESP32-S3 setup | https://docs.espressif.com/projects/esp-idf/en/v5.1.4/esp32s3/get-started/ | Espressif | ${retrieved} | 5.1.4 | - | Official setup reference |\n| LVGL documentation | https://docs.lvgl.io/8.4/ | LVGL project | ${retrieved} | 8.4 | bundled source in demo | GUI API |\n| SensorLib repository | https://github.com/lewisxhe/SensorLib | Lewis He | ${retrieved} | bundled 0.3.1 | bundled in demo archive | License retained in archive |\n\nCommunity repositories are link-only and recorded with branch, last push/update and license API fields in [community.md](community.md) and its local JSON metadata.\n`;
await Deno.writeTextFile(`${devicePath}/sources.md`, sources);

await Deno.writeTextFile(
  `${devicePath}/community.md`,
  `# Community resources\n\nRetrieved: ${retrieved}. These are secondary examples, not hardware authority. No repository source was copied because several repositories report no license or an indeterminate license. Snapshot metadata is retained at \`artifacts/source-snapshots/community-repositories-2026-08-21.json\`.\n\n| Repository | Default branch | Last push at retrieval | API license |\n|---|---|---|---|\n| https://github.com/VolosR/Knob18Meters | main | 2025-07-29 | none reported |\n| https://github.com/ihayri/ESP32-S3-1.8inch-Knob-Display-Development-Board | main | 2026-01-08 | none reported |\n| https://github.com/muness/roon-knob | master | 2026-08-21 | NOASSERTION |\n| https://github.com/0015/lvgl_kawaii_face | main | 2026-02-27 | MIT |\n| https://github.com/knobby-mtg/knobby-mtg-life-counter | main | 2026-07-30 | GPL-3.0 |\n| https://github.com/juggernautwins608/Waveshare-1.8-Magic-the-Gathering-Life-Counter | main | 2026-07-04 | MIT |\n| https://github.com/EmbeddedWizardGUI/ESP32-S3-Knob-Touch-LCD-1.8-EN | main | 2026-07-20 | none reported |\n| https://github.com/chris023/orion-waveshare-rotary-dial | main | 2026-08-05 | NOASSERTION |\n\nThe Waveshare wiki additionally links project videos/forums; follow the live wiki for those mutable references. Dates above are metadata snapshots, not endorsements or tested compatibility claims.\n`,
);

const componentRows = components.map((x) => `| [${x.title}](${x.path}/README.md) | ${x.kind} |`).join("\n");
await Deno.writeTextFile(
  `${root}/components/README.md`,
  `# Hardware components and interfaces\n\nResearch retrieved ${retrieved}. Manufacturer-specific parts use \`components/<manufacturer>/<part>/\`; unresolved or generic interfaces use \`components/generic/<category>/\`.\n\n| Record | Category |\n|---|---|\n${componentRows}\n`,
);
await Deno.mkdir(`${root}/devices/waveshare`, { recursive: true });
await Deno.writeTextFile(
  `${root}/devices/README.md`,
  `# Hardware devices\n\n- [Waveshare ESP32-S3-Knob-Touch-LCD-1.8](waveshare/esp32-s3-knob-touch-lcd-1.8/README.md), product ID 31623, retrieved ${retrieved}\n`,
);
await Deno.writeTextFile(
  `${root}/README.md`,
  `# Hardware research\n\nUmbrella for device and component research. Canonical real path is \`doc/hardware/\`; repository symlink \`docs\` exposes the same tree as \`docs/hardware/\`.\n\n- [Devices](devices/README.md)\n- [Components and interfaces](components/README.md)\n- [Artifact manifest](artifact-manifest.md)\n- [Inventory](inventory.txt)\n\nAll research in this initial set was retrieved ${retrieved}. Downloaded files are checksummed from local bytes; see each device/component record for provenance and caveats.\n`,
);
