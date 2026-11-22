package today.developing.pmconsole;

import android.app.PendingIntent;
import android.appwidget.AppWidgetManager;
import android.appwidget.AppWidgetProvider;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.text.format.DateFormat;
import android.view.View;
import android.widget.RemoteViews;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.Locale;
import java.util.TimeZone;
import org.json.JSONArray;
import org.json.JSONObject;

/**
 * The home-screen widget: how many tasks are in progress, how many are
 * queued, how many things need the owner, and what is being worked on
 * now. Each number opens its screen; the microphone starts dictation.
 *
 * <p>It is drawn from the last summary read, then the summary is read
 * again: every half hour, whenever the app is opened, and on a tap of
 * the refresh mark.
 */
public class StatusWidget extends AppWidgetProvider {
    static final String ACTION_REFRESH = "today.developing.pmconsole.REFRESH";
    /** A broadcast must be done well inside ten seconds. */
    private static final long BUDGET_MS = 8500;
    private static final long REQUEST_EVERY_MS = 30_000;
    private static volatile long lastRequest;

    @Override
    public void onReceive(Context context, Intent intent) {
        super.onReceive(context, intent);
        String action = intent.getAction();
        if (ACTION_REFRESH.equals(action) || AppWidgetManager.ACTION_APPWIDGET_UPDATE.equals(action)) {
            if (placed(context)) {
                read(context.getApplicationContext(), goAsync());
            }
        }
    }

    @Override
    public void onUpdate(Context context, AppWidgetManager manager, int[] ids) {
        draw(context);
    }

    /** Ask for the summary to be read again, at most twice a minute. */
    static void requestRefresh(Context context) {
        long now = System.currentTimeMillis();
        if (now - lastRequest < REQUEST_EVERY_MS || !placed(context)) {
            return;
        }
        lastRequest = now;
        context.sendBroadcast(new Intent(context, StatusWidget.class).setAction(ACTION_REFRESH));
    }

    private static boolean placed(Context context) {
        try {
            return AppWidgetManager.getInstance(context)
                            .getAppWidgetIds(new ComponentName(context, StatusWidget.class))
                            .length
                    > 0;
        } catch (RuntimeException unavailable) {
            return false;
        }
    }

    /** Read off the main thread, and let go of the broadcast in time whatever the network does. */
    private static void read(final Context app, final PendingResult pending) {
        final Thread worker =
                new Thread(
                        () -> {
                            try {
                                if (!Fleet.linked(app)) {
                                    Fleet.tryLink(app);
                                }
                                if (Fleet.linked(app)) {
                                    Fleet.refresh(app);
                                }
                                draw(app);
                            } catch (RuntimeException failed) {
                                // The widget keeps what it last showed.
                            }
                        },
                        "pm-widget");
        worker.start();
        new Thread(
                        () -> {
                            try {
                                worker.join(BUDGET_MS);
                            } catch (InterruptedException interrupted) {
                                Thread.currentThread().interrupt();
                            }
                            pending.finish();
                        },
                        "pm-widget-watch")
                .start();
    }

    /** Draw every placed widget from what this phone last read. */
    static void draw(Context context) {
        if (!placed(context)) {
            return;
        }
        RemoteViews views = new RemoteViews(context.getPackageName(), R.layout.widget_status);
        JSONObject summary = Fleet.cached(context);
        boolean linked = Fleet.linked(context);
        int flags = PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT;
        if (linked && summary != null) {
            views.setViewVisibility(R.id.widget_counts, View.VISIBLE);
            views.setTextViewText(R.id.widget_now_n, count(summary, "now", "count"));
            views.setTextViewText(R.id.widget_next_n, count(summary, "next", "count"));
            views.setTextViewText(R.id.widget_needs_n, count(summary, "needs", "total"));
            views.setTextViewText(R.id.widget_line, line(context, summary));
            views.setTextViewText(R.id.widget_asof, asOf(context, summary));
            views.setOnClickPendingIntent(R.id.widget_now, PendingIntent.getActivity(context, 1, Links.open(context, "now"), flags));
            views.setOnClickPendingIntent(R.id.widget_next, PendingIntent.getActivity(context, 2, Links.open(context, "next"), flags));
            views.setOnClickPendingIntent(R.id.widget_needs, PendingIntent.getActivity(context, 3, Links.open(context, "needs"), flags));
            views.setOnClickPendingIntent(R.id.widget_root, PendingIntent.getActivity(context, 4, Links.open(context, "now"), flags));
        } else {
            views.setViewVisibility(R.id.widget_counts, View.GONE);
            views.setTextViewText(
                    R.id.widget_line,
                    context.getString(
                            linked
                                    ? R.string.widget_loading
                                    : Fleet.pendingVerifier(context) != null
                                            ? R.string.widget_waiting
                                            : R.string.widget_unlinked));
            views.setTextViewText(R.id.widget_asof, "");
            Intent link = new Intent(context, LinkActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            views.setOnClickPendingIntent(R.id.widget_root, PendingIntent.getActivity(context, 5, link, flags));
        }
        Intent dictate = new Intent(context, DictateActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        views.setOnClickPendingIntent(R.id.widget_mic, PendingIntent.getActivity(context, 6, dictate, flags));
        Intent refresh = new Intent(context, StatusWidget.class).setAction(ACTION_REFRESH);
        views.setOnClickPendingIntent(R.id.widget_refresh, PendingIntent.getBroadcast(context, 7, refresh, flags));
        try {
            AppWidgetManager.getInstance(context).updateAppWidget(new ComponentName(context, StatusWidget.class), views);
        } catch (RuntimeException unavailable) {
            // No launcher to draw on right now.
        }
    }

    private static String count(JSONObject summary, String group, String field) {
        JSONObject object = summary.optJSONObject(group);
        int n = object == null ? 0 : object.optInt(field, 0);
        return n > 999 ? "999+" : String.valueOf(n);
    }

    /** What is being worked on now, or what comes next, in a line or two. */
    private static String line(Context context, JSONObject summary) {
        StringBuilder text = new StringBuilder();
        JSONObject now = summary.optJSONObject("now");
        JSONArray top = now == null ? null : now.optJSONArray("top");
        if (top != null && top.length() > 0) {
            JSONObject first = top.optJSONObject(0);
            text.append(first == null ? "" : first.optString("title", ""));
            String step = first == null ? "" : first.optString("step", "");
            if (!step.isEmpty()) {
                text.append("\n").append(context.getString(R.string.widget_step, step));
            }
        } else {
            JSONObject next = summary.optJSONObject("next");
            JSONArray queued = next == null ? null : next.optJSONArray("top");
            if (queued != null && queued.length() > 0) {
                text.append(context.getString(R.string.widget_next_up, queued.optString(0, "")));
            } else {
                text.append(context.getString(R.string.widget_idle));
            }
        }
        JSONObject asks = summary.optJSONObject("asks");
        int answered = asks == null ? 0 : asks.optInt("answered", 0);
        if (answered > 0) {
            text.append("\n").append(context.getResources().getQuantityString(R.plurals.widget_replies, answered, answered));
        }
        return text.toString();
    }

    /** The time the fleet's state was read, in the phone's own clock style. */
    private static String asOf(Context context, JSONObject summary) {
        long at = 0;
        String iso = summary.optString("at", "");
        if (!iso.isEmpty()) {
            try {
                SimpleDateFormat parser = new SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'", Locale.US);
                parser.setTimeZone(TimeZone.getTimeZone("UTC"));
                Date parsed = parser.parse(iso);
                at = parsed == null ? 0 : parsed.getTime();
            } catch (Exception unparsed) {
                at = 0;
            }
        }
        if (at == 0) {
            at = Fleet.fetchedAt(context);
        }
        return at == 0 ? "" : DateFormat.getTimeFormat(context).format(new Date(at));
    }
}
