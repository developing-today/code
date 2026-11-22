// Phone push for the queue transport alert: a Telegram message from
// Drew's existing OpenClaw bot (@shed_openclaw_bot) to his DM. The Bot
// API's sendMessage is outbound only, so it does not disturb the
// gateway's own polling, and it works while the laptop is off.
export function makeTelegramNotifier(botToken, chatId, fetchImpl = fetch) {
  return async function notify(text) {
    const res = await fetchImpl(`https://api.telegram.org/bot${botToken}/sendMessage`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ chat_id: chatId, text, disable_web_page_preview: true }),
    });
    const body = await res.text();
    let ok = false;
    try {
      ok = JSON.parse(body).ok === true;
    } catch {
      ok = false;
    }
    // Never echo the URL: it carries the bot token.
    if (!res.ok || !ok) throw new Error(`Telegram sendMessage -> HTTP ${res.status}: ${body.slice(0, 200)}`);
  };
}
