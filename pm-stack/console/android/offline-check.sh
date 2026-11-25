#!/usr/bin/env bash
# offline-check.sh — drive the PM console Android app on an emulator
# against the demo edge, with the network off and on again.
#
# Usage:
#   offline-check.sh --apk <apk> --edge-url <url> --act-log <file>
#     [--answer-label <text>] [--question-text <text>]
#     [--next-question-text <text>]
#
# The caller starts the emulator and the demo edge first: the APK is
# built against the emulator's view of the edge
# (PM_CONSOLE_SITE=http://10.0.2.2:<port>), the edge answers at
# --edge-url from this machine, and --act-log is the file the edge
# appends every action it runs to (PM_CONSOLE_DEMO_ACT_LOG).
#
# The check, in order:
#   1. the app opens against the demo edge and shows the demo's question
#   2. with the network off, the app opened again from its icon shows
#      the same question from what the device kept
#   3. the question is answered while offline, the app is swiped away,
#      and the network comes back
#   4. the demo edge saw that answer exactly once, and reopening the
#      app no longer shows the question
#
# Needs: adb, python3, uiautomator on the emulator. Exits non-zero
# naming the step that failed, with the last screen's text on stderr.
# A step that only sets the scene (Chrome's first-run screen, say)
# never fails the check by itself: the wait for the app's own text is
# what decides.
set -euo pipefail

APK=""
EDGE_URL=""
ACT_LOG=""
ANSWER_LABEL="Build new attachment storage"
QUESTION_TEXT="For S7 attachments and multimedia, build new storage, or reuse a mechanism outside pm-stack that you name?"
NEXT_QUESTION_TEXT="Which checks must pass before a batch merges?"
PKG="today.developing.pmconsole"
LAUNCHER="today.developing.pmconsole/com.google.androidbrowserhelper.trusted.LauncherActivity"

usage() {
  echo "usage: offline-check.sh --apk <apk> --edge-url <url> --act-log <file> [--answer-label <text>] [--question-text <text>] [--next-question-text <text>]" >&2
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
  --apk)
    APK="${2:-}"
    shift 2
    ;;
  --edge-url)
    EDGE_URL="${2:-}"
    shift 2
    ;;
  --act-log)
    ACT_LOG="${2:-}"
    shift 2
    ;;
  --answer-label)
    ANSWER_LABEL="${2:-}"
    shift 2
    ;;
  --question-text)
    QUESTION_TEXT="${2:-}"
    shift 2
    ;;
  --next-question-text)
    NEXT_QUESTION_TEXT="${2:-}"
    shift 2
    ;;
  *)
    echo "offline-check: unknown argument: $1" >&2
    usage
    ;;
  esac
done
[ -n "$APK" ] && [ -n "$EDGE_URL" ] && [ -n "$ACT_LOG" ] || usage
[ -f "$APK" ] || {
  echo "offline-check: no APK at $APK" >&2
  exit 2
}

# uiautomator follows the XML with a line of its own ("UI hierchary
# dumped to: ..."), so the parses below keep only the document, from
# "<hierarchy" through "</hierarchy>": with the trailer left on, every
# parse fails and every screen reads as empty.
# The screen as the emulator sees it, one text per line.
screen_text() {
  adb exec-out uiautomator dump /dev/tty 2>/dev/null | python3 -c '
import sys, xml.etree.ElementTree as ET
raw = sys.stdin.buffer.read()
start = raw.find(b"<hierarchy")
if start < 0:
    sys.exit(0)
end = raw.find(b"</hierarchy>", start)
root = ET.fromstring(raw[start:end + len(b"</hierarchy>") if end >= 0 else None].decode("utf-8", "replace"))
for node in root.iter("node"):
    text = node.get("text") or ""
    if text:
        print(text)
'
}

# The centre of the first element whose text is exactly the needle,
# or nothing.
tap_xy() {
  adb exec-out uiautomator dump /dev/tty 2>/dev/null | python3 -c '
import sys, xml.etree.ElementTree as ET
needle = sys.argv[1]
raw = sys.stdin.buffer.read()
start = raw.find(b"<hierarchy")
if start < 0:
    sys.exit(1)
end = raw.find(b"</hierarchy>", start)
root = ET.fromstring(raw[start:end + len(b"</hierarchy>") if end >= 0 else None].decode("utf-8", "replace"))
for node in root.iter("node"):
    if (node.get("text") or "") == needle:
        bounds = node.get("bounds") or ""
        nums = bounds.replace("[", " ").replace("]", " ").replace(",", " ").split()
        if len(nums) == 4:
            x1, y1, x2, y2 = (int(n) for n in nums)
            print((x1 + x2) // 2, (y1 + y2) // 2)
            sys.exit(0)
sys.exit(1)
' "$1"
}

dump_for_log() {
  echo "--- screen ---" >&2
  screen_text >&2 || true
  echo "--- end screen ---" >&2
  echo "--- foreground: $(adb shell dumpsys activity activities 2>/dev/null | grep -m 1 'topResumedActivity' | tr -d '[:space:]') ---" >&2
  echo "--- logcat (app, launcher and activity lines) ---" >&2
  adb logcat -d -t 8000 2>/dev/null | grep -aE 'FATAL EXCEPTION|pmconsole|LauncherActivity|TwaLauncher|CustomTabs|ActivityTaskManager' | grep -avE 'uiautomator|tachyon' | tail -40 >&2 || true
  echo "--- end logcat ---" >&2
  echo "--- activity events for the app ---" >&2
  adb logcat -b events -d 2>/dev/null | grep -aE 'am_create_activity|am_finish_activity|am_destroy_activity|am_proc_died|am_crash|am_anr' | grep -a 'pmconsole' | tail -20 >&2 || true
  echo "--- end activity events ---" >&2
}

fail() {
  echo "offline-check: FAILED: $*" >&2
  dump_for_log
  exit 1
}

wait_text() {
  local needle="$1" timeout="${2:-60}" i=0 screen=""
  while [ "$i" -lt "$timeout" ]; do
    screen="$(screen_text)"
    if printf '%s' "$screen" | grep -qF "$needle"; then return 0; fi
    # A system "isn't responding" dialog — the launcher's, under a
    # loaded emulator — is scenery standing between the check and
    # the app, not the app itself: tell it to wait and keep waiting
    # for the app's own text, which is what decides.
    if printf '%s' "$screen" | grep -qF "isn't responding"; then
      tap_text "Wait" || true
    fi
    sleep 2
    i=$((i + 2))
  done
  return 1
}

wait_no_text() {
  local needle="$1" timeout="${2:-30}" i=0
  while [ "$i" -lt "$timeout" ]; do
    if ! screen_text | grep -qF "$needle"; then return 0; fi
    sleep 2
    i=$((i + 2))
  done
  return 1
}

tap_text() {
  local needle="$1" xy
  xy="$(tap_xy "$needle")" || return 1
  adb shell input tap "$xy"
}

# One action arriving at the demo edge, counted from its log.
count_answer() {
  python3 -c '
import json, sys
kind, qid, option = sys.argv[1], sys.argv[2], sys.argv[3]
n = 0
try:
    with open(sys.argv[4]) as f:
        for line in f:
            try:
                e = json.loads(line)
            except ValueError:
                continue
            a = e.get("args") or {}
            if e.get("kind") == kind and a.get("qid") == qid and a.get("option") == option:
                n += 1
except FileNotFoundError:
    pass
print(n)
' "question.answer" "q7" "opt-1" "$ACT_LOG"
}

airplane() {
  local want="$1" # on or off
  if [ "$want" = "on" ]; then
    adb shell cmd connectivity airplane-mode enable >/dev/null
  else
    adb shell cmd connectivity airplane-mode disable >/dev/null
  fi
  local expect="0"
  [ "$want" = "on" ] && expect="1"
  local i=0 got=""
  while [ "$i" -lt 30 ]; do
    got="$(adb shell settings get global airplane_mode_on 2>/dev/null | tr -d '[:space:]')"
    [ "$got" = "$expect" ] && return 0
    sleep 2
    i=$((i + 2))
  done
  return 1
}

# Open the app at its launcher activity. This used monkey, the way
# the home screen does, until run 45558906810: monkey's launches
# never brought the app up at all (its output was thrown away, so
# it failed silently), the waits sat on the launcher, and the
# launcher's own long-press menu ended up covering the screen. An
# explicit start names the activity the icon names, waits for it,
# and says on the log what happened.
launch_from_icon() {
  adb shell am start -W -n "$LAUNCHER" 2>&1 | grep -v '^$' || true
}

# Open the app and wait for its text, opening it again while the
# text has not appeared: a launch fired while the launcher is still
# settling is lost without a trace — run 45557841422 waited out its
# whole first wait on the home screen — and one lost launch is the
# launcher failing, not the app.
launch_and_wait_text() {
  local needle="$1" timeout="${2:-60}" i=0
  while [ "$i" -lt "$timeout" ]; do
    launch_from_icon
    if wait_text "$needle" 20; then return 0; fi
    i=$((i + 20))
  done
  return 1
}

echo "offline-check: waiting for the emulator"
adb wait-for-device
i=0
while [ "$i" -lt 120 ]; do
  [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '[:space:]')" = "1" ] && break
  sleep 5
  i=$((i + 5))
done
[ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '[:space:]')" = "1" ] || fail "emulator did not finish booting"

# Chrome's first-run screen would stand between the app and its page:
# tell it not to show, before it first starts.
adb shell 'echo "chrome --no-first-run --disable-fre" > /data/local/tmp/chrome-command-line' 2>/dev/null || true
adb shell am force-stop com.android.chrome 2>/dev/null || true

echo "offline-check: installing the APK"
adb install -r -g "$APK" || fail "could not install $APK"
adb shell am force-stop "$PKG" 2>/dev/null || true

echo "offline-check: 1. the app opens against the demo edge"
launch_and_wait_text "Needs me" 120 || fail "the app did not open to the console (no 'Needs me' tab)"
tap_text "Needs me" || fail "could not open the Needs me tab"
wait_text "$QUESTION_TEXT" 60 || fail "the demo question is not shown"

echo "offline-check: 2. with no connection, opened again it shows what it kept"
airplane on || fail "could not turn aeroplane mode on"
launch_and_wait_text "$QUESTION_TEXT" 60 || fail "opened with no connection, the app does not show the question it last saw"

echo "offline-check: 3. answered while offline, app swiped away, network back"
tap_text "$ANSWER_LABEL" || fail "could not tap the answer '$ANSWER_LABEL'"
wait_no_text "$QUESTION_TEXT" 30 || fail "the answered question is still listed"
# The answer is only written to the outbox when its 4.5-second undo
# window runs out (app.js: later), so the app stays open past that
# before it is swiped away: swiping sooner would lose the answer with
# the page, and the check would be testing the undo timer, not the
# sync. Eight seconds leaves room for the write and the sync to be
# registered.
sleep 8
# Swiped away: the task leaves recents, so the page is gone and only
# Chrome's worker is left to send what was kept.
adb shell input keyevent KEYCODE_APP_SWITCH
sleep 2
adb shell input swipe 540 1300 540 200 300
sleep 2
if adb shell dumpsys activity activities 2>/dev/null | grep -q "$PKG"; then
  adb shell am force-stop "$PKG" 2>/dev/null || true
  sleep 2
fi
if adb shell pidof "$PKG" >/dev/null 2>&1; then
  fail "the app is still running after being swiped away"
fi
airplane off || fail "could not turn aeroplane mode off"

echo "offline-check: 4. the edge saw the answer exactly once"
i=0
seen="0"
while [ "$i" -lt 60 ]; do
  seen="$(count_answer)"
  [ "$seen" = "1" ] && break
  sleep 2
  i=$((i + 2))
done
[ "$seen" = "1" ] || fail "the demo edge saw the answer $seen times, not once (log: $ACT_LOG)"
launch_and_wait_text "Needs me" 60 || fail "the app did not reopen after the answer was sent"
tap_text "Needs me" || fail "could not open the Needs me tab again"
wait_text "$NEXT_QUESTION_TEXT" 60 || fail "the Needs me list did not load after the answer was sent"
if screen_text | grep -qF "$QUESTION_TEXT"; then
  fail "the answered question is back in the list after being sent"
fi

echo "offline-check: passed — the app opened offline, the answer made offline was sent once with the app swiped away, and the question is gone"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "The Android app opened with no connection, showed the question it last saw, and the answer made offline reached the demo edge exactly once after the app was swiped away and the network came back."
  } >>"$GITHUB_STEP_SUMMARY"
fi
