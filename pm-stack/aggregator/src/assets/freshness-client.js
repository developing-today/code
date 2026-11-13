// PM aggregator freshness client (#3976, on the #3978 stream):
// the browser side of event-driven freshness, with the degraded-mode
// fallback the design requires.
//
// Live mode: one EventSource on /freshness/stream. A `freshness`
// notice (or a `reset`) means "reread canonical data" — the notice
// carries no payload — so the client dispatches the same `dashrefresh`
// event the dashboard's own actions use: fragment containers re-GET
// their canonical fragment and the view updates in place.
//
// Degraded mode (policy: src/freshness-fallback.ts, SSE_DOWN_REFRESH_MS
// = 5 minutes — the collector's cadence): while the stream is down,
// poll the machine twin /api/freshness-events.json on the 5-minute
// cadence and dispatch the same refresh on every poll (the policy's
// cadence refresh, not only when the notice cursor moved). The
// moment the stream reopens, polling stops. Neither path reloads the
// page, so view state survives both.
(function () {
  if (!("EventSource" in window)) return;
  var REFRESH_MS = 5 * 60_000;
  var cursor = 0;
  var downAt = null;
  var lastRefreshAt = Date.now();
  var timer = null;

  function refresh() {
    lastRefreshAt = Date.now();
    window.dispatchEvent(new CustomEvent("dashrefresh"));
  }

  function schedule() {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    if (downAt === null) return;
    var anchor = Math.max(downAt, lastRefreshAt);
    var delay = Math.max(0, anchor + REFRESH_MS - Date.now());
    timer = setTimeout(poll, delay);
  }

  async function poll() {
    timer = null;
    if (downAt === null) return;
    try {
      var res = await fetch("/api/freshness-events.json?after=" + cursor, {
        headers: { accept: "application/json" },
      });
      if (res.ok) {
        var envelope = await res.json();
        if (envelope && typeof envelope.cursor === "number") {
          cursor = envelope.cursor;
        }
        // The degraded-mode policy (src/freshness-fallback.ts) is a
        // canonical refresh on the cadence itself, not only when the
        // notice cursor moved: a fragment reread that fails
        // transiently after a notice was consumed must not strand the
        // view until some later notice happens to arrive. Refresh on
        // every successful poll while the stream is down.
        refresh();
      }
    } catch (e) {
      /* a failed poll is retried on the same cadence; the data on
         screen stays visibly as-is, never silently marked fresh */
    }
    lastRefreshAt = Date.now();
    schedule();
  }

  var stream = new EventSource("/freshness/stream");
  stream.addEventListener("open", function () {
    downAt = null;
    schedule();
  });
  stream.addEventListener("error", function () {
    if (downAt === null) downAt = Date.now();
    schedule();
  });
  stream.addEventListener("freshness", function (ev) {
    if (ev && typeof ev.lastEventId === "string" && /^\d+$/.test(ev.lastEventId)) {
      cursor = Number(ev.lastEventId);
    }
    refresh();
  });
  stream.addEventListener("reset", function () {
    // The retained journal no longer covers our cursor: resync from
    // the JSON twin, then refresh the whole view once.
    fetch("/api/freshness-events.json?after=0", { headers: { accept: "application/json" } })
      .then(function (r) {
        return r.ok ? r.json() : null;
      })
      .then(function (envelope) {
        if (envelope && typeof envelope.cursor === "number") cursor = envelope.cursor;
      })
      .catch(function () {})
      .finally(refresh);
  });
})();
