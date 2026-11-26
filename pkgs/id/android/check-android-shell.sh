#!/usr/bin/env bash
# Structural smoke test for the Android shell (no SDK or emulator needed).
# Argument contract: takes no arguments; reject them before reading files.
set -euo pipefail
if [ "$#" -gt 0 ]; then
  echo "usage: check-android-shell.sh (takes no arguments)" >&2
  echo "check-android-shell.sh: usage error: unexpected argument: $1" >&2
  exit 2
fi
cd "$(dirname "$0")"
python3 -I - <<'PY'
import pathlib
import xml.etree.ElementTree as ET

root = pathlib.Path(".")
manifest = ET.parse(root / "app/src/main/AndroidManifest.xml").getroot()
ns = "{http://schemas.android.com/apk/res/android}"
application = manifest.find("application")
assert application is not None, "application element is required"
assert application.get(ns + "allowBackup") == "false", "the PoC must not back up connection inputs"
activity = application.find("activity")
assert activity is not None and activity.get(ns + "name") == ".MainActivity"
assert activity.get(ns + "exported") == "true"
assert activity.find("intent-filter/action").get(ns + "name") == "android.intent.action.MAIN"
permissions = [e.get(ns + "name") for e in manifest if e.tag.endswith("uses-permission")]
assert permissions == ["android.permission.INTERNET"], "PoC may request only the normal Internet permission"
assert not any("VPN" in p for p in permissions), "PoC must not request VPN permission"
assert (root / "settings.gradle.kts").is_file()
assert (root / "build.gradle.kts").is_file()
assert (root / "app/build.gradle.kts").is_file()
source = (root / "app/src/main/java/org/developingtoday/id/MainActivity.kt").read_text()
assert "class MainActivity" in source
assert "Not connected" in source, "shell must not imply a live relay"
bridge = (root / "app/src/main/java/org/developingtoday/id/NativeBridge.kt").read_text()
assert "System.loadLibrary(\"id_android_remote\")" in bridge
assert "external fun nativeStart" in bridge and "external fun nativeSnapshot" in bridge
gradle = (root / "app/build.gradle.kts").read_text()
assert '"cargo"' in gradle and '"ndk"' in gradle, "Gradle must build the Rust bridge"
native = (root / "../crates/id-android-remote/src/android_jni.rs").read_text()
core = (root / "../crates/id-android-remote/src/lib.rs").read_text()
assert "nativeStart" in native and "nativeSnapshot" in native and "nativeStop" in native
assert "WorldClient" in core and "WorldProtocol" not in core + native, "the Android process must be a client only"
assert "VpnService" not in source and "VpnService" not in manifest.attrib
print("PASS: Android remote-drive shell and Rust JNI client are wired; Internet-only permission; no VPN or implicit host")
PY
