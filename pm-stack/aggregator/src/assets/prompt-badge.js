// PM aggregator prompt unread badge (#10035 S6, design #9539 §4):
// the nav carries a hidden "✉ n unread" link to /chat on every page;
// this fills it from /api/prompt/unread-count and unhides it only
// when there is something unread. Failures stay silent — a missing
// badge must never break a page.
(function () {
  var badge = document.querySelector("[data-prompt-unread]");
  if (!badge) return;
  fetch("/api/prompt/unread-count")
    .then(function (r) {
      return r.ok ? r.json() : null;
    })
    .then(function (data) {
      var count = data && typeof data.count === "number" ? data.count : 0;
      if (count <= 0) return;
      var slot = badge.querySelector("[data-prompt-unread-count]");
      if (slot) slot.textContent = String(count);
      badge.hidden = false;
    })
    .catch(function () {
      /* the badge is an indicator, never a page dependency */
    });
})();
