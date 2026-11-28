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
SITE_URL=""
ANSWER_LABEL="Build new attachment storage"
QUESTION_TEXT="For S7 attachments and multimedia, build new storage, or reuse a mechanism outside pm-stack that you name?"
NEXT_QUESTION_TEXT="Which checks must pass before a batch merges?"
PKG="today.developing.pmconsole"
LAUNCHER="today.developing.pmconsole/com.google.androidbrowserhelper.trusted.LauncherActivity"

usage() {
  echo "usage: offline-check.sh --apk <apk> --edge-url <url> --act-log <file> [--site-url <url>] [--answer-label <text>] [--question-text <text>] [--next-question-text <text>]" >&2
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
  --site-url)
    SITE_URL="${2:-}"
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
  timeout 120 adb exec-out uiautomator dump /dev/tty 2>/dev/null | python3 -c '
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
  timeout 120 adb exec-out uiautomator dump /dev/tty 2>/dev/null | python3 -c '
import sys, xml.etree.ElementTree as ET
needle = sys.argv[1]
raw = sys.stdin.buffer.read()
start = raw.find(b"<hierarchy")
if start < 0:
    sys.exit(1)
end = raw.find(b"</hierarchy>", start)
root = ET.fromstring(raw[start:end + len(b"</hierarchy>") if end >= 0 else None].decode("utf-8", "replace"))
def box_of(node):
    bounds = node.get("bounds") or ""
    nums = bounds.replace("[", " ").replace("]", " ").replace(",", " ").split()
    if len(nums) != 4:
        return None
    x1, y1, x2, y2 = (int(n) for n in nums)
    if x2 <= x1 or y2 <= y1:
        return None
    return x1, y1, x2, y2

def tap_point(node):
    box = box_of(node)
    if box is None:
        return None
    x1, y1, x2, y2 = box
    return node.get("bounds"), (x1 + x2) // 2, (y1 + y2) // 2

def covers(box, cx, cy):
    x1, y1, x2, y2 = box
    return x1 <= cx <= x2 and y1 <= cy <= y2

def label_of(node):
    return node.get("text") or node.get("content-desc") or ""

nodes = list(root.iter("node"))

def occluded_by(cx, cy, self_box):
    # A tap lands on the topmost view at its point: where the target
    # shares its centre with another tappable node — the bottom tab
    # bar of the app over page content scrolled beneath it — the tap
    # lands on the cover, not the target. Ancestors hold the whole
    # target inside themselves and are not covers: only a node with
    # its own label, a different box, and the centre but not the
    # whole target inside it counts. The smaller-box rule keeps the
    # reverse direction working too: tapping the tab bar itself is
    # never blocked by the bigger content behind it.
    sx1, sy1, sx2, sy2 = self_box
    sarea = (sx2 - sx1) * (sy2 - sy1)
    for other in nodes:
        if not (other.get("clickable") == "true" or (other.get("text") or other.get("content-desc"))):
            continue
        obox = box_of(other)
        if obox is None or obox == self_box:
            continue
        if not covers(obox, cx, cy):
            continue
        ox1, oy1, ox2, oy2 = obox
        if ox1 <= sx1 and oy1 <= sy1 and ox2 >= sx2 and oy2 >= sy2:
            continue
        if (ox2 - ox1) * (oy2 - oy1) >= sarea:
            continue
        return other
    return None

# A match only counts with a real box: the screen read holds the
# whole page, and a node off the screen reports bounds [0,0][0,0] —
# the answer tap of run 45578833900 matched such a node and tapped
# the corner of the screen. First pass: the label at the start of
# the node text. Second pass: the label anywhere in it — a
# recommended option names its button with the word RECOMMENDED in
# front of the label, so the answer the check taps never starts
# the text of its button.
occluded = None
for mode in ("prefix", "contains"):
    for node in nodes:
        label = label_of(node)
        hit = label.startswith(needle) if mode == "prefix" else needle in label
        if not hit:
            continue
        got = tap_point(node)
        if got is None:
            continue
        bounds, cx, cy = got
        cover = occluded_by(cx, cy, box_of(node))
        if cover is not None:
            occluded = (mode, label, bounds, cx, cy, label_of(cover), cover.get("bounds"))
            continue
        print("tap_xy: %r -> (%s) text=%r bounds=%s tap=%d,%d" % (
            needle, mode, label, bounds, cx, cy), file=sys.stderr)
        print(cx, cy)
        sys.exit(0)
if occluded is not None:
    mode, label, bounds, cx, cy, clabel, cbounds = occluded
    print("tap_xy: %r -> (%s) text=%r bounds=%s tap=%d,%d OCCLUDED by text=%r bounds=%s" % (
        needle, mode, label, bounds, cx, cy, clabel, cbounds), file=sys.stderr)
    sys.exit(2)
# No tappable node matched: say what the screen did hold, so a miss
# is diagnosable from the run log (run 45574682050 missed a tab five
# times that the failure dump, seconds later, showed plainly). Nodes
# that name the needle but report no box are counted separately: a
# visible label with bounds [0,0][0,0] is rendered off-screen, and no
# tap can reach it without scrolling.
seen = [n.get("text") for n in nodes if n.get("text")]
boxless = sum(1 for n in nodes if needle in label_of(n) and tap_point(n) is None)
print("tap_xy: no tappable node names %r (label matches with no box: %d); screen texts: %s" % (needle, boxless, seen[:40]), file=sys.stderr)
sys.exit(1)
' "$1"
}

# The device reads here and in the screen helpers carry timeouts:
# in run 45568729971 the check failed in its second minute and the
# dump's own read then hung on the wedged emulator for forty, until
# the run was cancelled — a diagnostic must never hold the run.
dump_for_log() {
  # A system dialog — the launcher's "isn't responding", under this
  # emulator's load — may be all that covers the app at the moment
  # of failure (run 45563986699's dump showed only the dialog, with
  # the app's browser tab already open behind it). Dismiss it the
  # way wait_text does and read the screen again, so the dump names
  # the app's own state, not the dialog's.
  if screen_text | grep -qF "isn't responding"; then
    dismiss_anr
    sleep 2
  fi
  echo "--- screen ---" >&2
  screen_text >&2 || true
  echo "--- end screen ---" >&2
  echo "--- foreground: $(timeout 60 adb shell dumpsys activity activities 2>/dev/null | grep -m 1 'topResumedActivity' | tr -d '[:space:]') ---" >&2
  echo "--- logcat (app, launcher and activity lines) ---" >&2
  timeout 60 adb logcat -d -t 8000 2>/dev/null | grep -aE 'FATAL EXCEPTION|pmconsole|LauncherActivity|TwaLauncher|CustomTabs|ActivityTaskManager' | grep -avE 'uiautomator|tachyon' | tail -40 >&2 || true
  echo "--- end logcat ---" >&2
  echo "--- activity events for the app ---" >&2
  timeout 60 adb logcat -b events -d 2>/dev/null | grep -aE 'am_create_activity|am_finish_activity|am_destroy_activity|am_proc_died|am_crash|am_anr' | grep -a 'pmconsole' | tail -20 >&2 || true
  echo "--- end activity events ---" >&2
}

fail() {
  echo "offline-check: FAILED: $*" >&2
  dump_for_log
  exit 1
}

# Dismiss a system "isn't responding" dialog. Tapping Wait does not
# clear the launcher's: in run 45580772520 five landed Wait taps left
# it standing, the launcher wedged under the emulator's load and the
# dialog back at once each time. Stopping the launcher clears it —
# the system starts the launcher again when it is wanted, and the
# check never needs it: the app is started by name, and the recents
# screen is the system's, not the launcher's. The Wait tap stays for
# a dialog that is some other app's.
dismiss_anr() {
  adb shell am force-stop com.google.android.apps.nexuslauncher >/dev/null 2>&1 || true
  tap_text "Wait" || true
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
      dismiss_anr
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
  local needle="$1" xy rc
  xy="$(tap_xy "$needle")"
  rc=$?
  # Exit 2 names an occluded target: its centre sits under another
  # tappable node — the answer button scrolled beneath the app's own
  # bottom tab bar, where the tap would land on the tab and sail to
  # another list instead of answering (runs 46042225102 and
  # 46046959172: the tap logged a button centre inside the Needs-me
  # tab's bounds, the question left the screen by navigation, and
  # the edge saw nothing). The caller scrolls it clear and looks again.
  if [ "$rc" -eq 2 ]; then return 2; fi
  [ "$rc" -eq 0 ] || return 1
  adb shell input tap "$xy"
}

# Waits for words, scrolling the page down between looks: a list
# can run past the fold, and a screen read only holds what is on
# the screen. A swipe up, a breath, another look.
wait_text_scrolling() {
  local tries=0 texts
  until texts="$(screen_text)" && printf '%s' "$texts" | grep -qF "$1"; do
    tries=$((tries + 1))
    [ "$tries" -gt 10 ] && return 1
    # Say where the page stands, so a stuck scroll is diagnosable:
    # run 45575646271 scrolled ten times and the failure dump still
    # showed the top of the page.
    echo "offline-check: scroll $tries: page shows '$(printf '%s' "$texts" | head -3 | tr '\n' ' ')' … '$(printf '%s' "$texts" | tail -2 | tr '\n' ' ')'" >&2
    adb shell input swipe 540 1600 540 600 300 >/dev/null 2>&1 || true
    sleep 2
  done
}

# Taps a chip that sits off the right end of a sideways strip: the
# strip only moves under a sideways gesture that starts on it, so
# each miss swipes left from a visible neighbour chip's own row,
# pulling the target in from the right. Six looks altogether.
reveal_right_of() {
  local anchor="$1" target="$2" tries=0 rc=1 axy ax ay
  while ! tap_text "$target"; do
    rc=$?
    tries=$((tries + 1))
    [ "$tries" -gt 8 ] && return 1
    # Occluded is the bottom tab bar over page content, not the
    # strip: scroll the page so the target moves clear of it.
    if [ "$rc" -eq 2 ]; then
      adb shell input swipe 540 1500 540 700 300 >/dev/null 2>&1 || true
      sleep 2
      continue
    fi
    # Where the neighbour chip itself reports no box, the whole
    # strip is out of view above the fold: swipe down to bring the
    # top of the page back before trying the strip.
    if ! axy="$(tap_xy "$anchor")"; then
      adb shell input swipe 540 700 540 1400 300 >/dev/null 2>&1 || true
      sleep 2
      continue
    fi
    ax="${axy% *}"
    ay="${axy#* }"
    adb shell input swipe "$((ax + 250))" "$ay" "$((ax - 250))" "$ay" 300 >/dev/null 2>&1 || true
    sleep 2
  done
}

# Taps, retrying while the page settles: a single look can race the
# rendering — in run 45567951938 the tab bar was on the screen in the
# failure dump seconds after the stage-2 tap's own look had missed
# it. Five looks, a breath between.
tap_text_retry() {
  local tries=0 rc=1
  until tap_text "$1"; do
    rc=$?
    tries=$((tries + 1))
    [ "$tries" -ge 5 ] && return 1
    # An occluded target sits under another tappable node — the
    # answer button beneath the bottom tab bar. Scroll the page so
    # the target moves clear of the bar, then look again.
    if [ "$rc" -eq 2 ]; then
      echo "offline-check: tap target '$1' is occluded, scrolling it clear (try $tries)" >&2
      adb shell input swipe 540 1500 540 700 300 >/dev/null 2>&1 || true
      sleep 2
      continue
    fi
    # A system dialog may be all that covers the app — in run
    # 45579940065 the launcher's "isn't responding" dialog came back
    # after the dismissal in wait_text, and every tap look saw only
    # it. Dismiss it the same way and look again.
    if screen_text | grep -qF "isn't responding"; then
      dismiss_anr
      sleep 2
    fi
    sleep 3
  done
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
# tell it not to show, before it first starts. And where the app is
# opened from a plain-http address (the emulator's name for this
# machine), tell Chrome to treat that one address as secure: the
# app's worker — the part that sends what was done offline once the
# connection is back, which stage 4 tests — only exists on secure
# origins, and without this the check tests a browser with no worker
# in it (run 45582639147: the answer was kept, and never sent). On
# the app's real https address nothing is added.
CHROME_FLAGS="chrome --no-first-run --disable-fre"
case "$SITE_URL" in
http://*) CHROME_FLAGS="$CHROME_FLAGS --unsafely-treat-insecure-origin-as-secure=$SITE_URL" ;;
esac
adb shell "echo \"$CHROME_FLAGS\" > /data/local/tmp/chrome-command-line" 2>/dev/null || true
adb shell am force-stop com.android.chrome 2>/dev/null || true

echo "offline-check: installing the APK"
adb install -r -g "$APK" || fail "could not install $APK"
adb shell am force-stop "$PKG" 2>/dev/null || true

echo "offline-check: 1. the app opens against the demo edge"
launch_and_wait_text "Needs me" 120 || fail "the app did not open to the console (no 'Needs me' tab)"
tap_text_retry "Needs me" || fail "could not open the Needs me tab"
# The Needs me page opens on whichever list leads; the questions are
# their own segment of it, behind the "Questions" button (app.js:
# viewNeeds' seg buttons), not further down the page — ten scrolls
# in run 45576589239 moved nothing because there was nothing to
# scroll to.
tap_text_retry "Questions" || fail "could not open the Questions segment"
wait_text_scrolling "$QUESTION_TEXT" || fail "the demo question is not shown"

echo "offline-check: 2. with no connection, opened again it shows what it kept"
airplane on || fail "could not turn aeroplane mode on"
if ! launch_and_wait_text "$QUESTION_TEXT" 60; then
  launch_from_icon
  tap_text_retry "Questions" || true
  wait_text_scrolling "$QUESTION_TEXT" || fail "opened with no connection, the app does not show the question it last saw"
fi

echo "offline-check: 3. answered while offline, app swiped away, network back"
wait_text_scrolling "$ANSWER_LABEL" || fail "the answer '$ANSWER_LABEL' is not shown"
tap_text_retry "$ANSWER_LABEL" || fail "could not tap the answer '$ANSWER_LABEL'"
wait_no_text "$QUESTION_TEXT" 30 || fail "the answered question is still listed"
# The question leaving the screen is not yet proof it was answered:
# a tap that lands on the bottom tab bar instead of the button sails
# to another list, and the question is gone because the list is —
# while nothing was kept and stage 4 sees nothing (runs 46042225102
# and 46046959172). The answered list still holds its neighbours at
# the same scroll position, so require one to still be there before
# swiping the app away. No scrolling here: the list has not moved,
# and scrolling could sail past the neighbour.
wait_text "$NEXT_QUESTION_TEXT" 15 || fail "the answer tap may have left the Questions list instead of answering (neighbour question not shown)"
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
if timeout 60 adb shell dumpsys activity activities 2>/dev/null | grep -q "$PKG"; then
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
tap_text_retry "Needs me" || fail "could not open the Needs me tab again"
# The Needs page opens on whichever list leads, and after the answer
# the hands list leads: the segment chips sit in a sideways strip
# with Questions off its right end. Pull it in from the Reviews chip.
reveal_right_of "Reviews" "Questions" || fail "could not reopen the Questions segment"
wait_text_scrolling "$NEXT_QUESTION_TEXT" || fail "the Needs me list did not load after the answer was sent"
if screen_text | grep -qF "$QUESTION_TEXT"; then
  fail "the answered question is back in the list after being sent"
fi

echo "offline-check: passed — the app opened offline, the answer made offline was sent once with the app swiped away, and the question is gone"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "The Android app opened with no connection, showed the question it last saw, and the answer made offline reached the demo edge exactly once after the app was swiped away and the network came back."
  } >>"$GITHUB_STEP_SUMMARY"
fi
