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
activity = application.find("activity")
assert activity is not None and activity.get(ns + "name") == ".MainActivity"
assert activity.get(ns + "exported") == "true"
assert activity.find("intent-filter/action").get(ns + "name") == "android.intent.action.MAIN"
assert not any(e.tag.endswith("uses-permission") for e in manifest), "PoC must not request permissions"
assert (root / "settings.gradle.kts").is_file()
assert (root / "build.gradle.kts").is_file()
assert (root / "app/build.gradle.kts").is_file()
source = (root / "app/src/main/java/org/developingtoday/id/MainActivity.kt").read_text()
assert "class MainActivity" in source
assert "Not connected" in source, "shell must not imply a live relay"
print("PASS: Android launcher shell is configured; no permissions requested; disconnected state is explicit")
PY
