package today.developing.pmconsole;

import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import com.google.androidbrowserhelper.trusted.LauncherActivity;

/** Where things are in the app, and how to open them from outside it. */
final class Links {
    /** Longest text handed to the app in a link. */
    private static final int MAX_TEXT = 6000;

    private Links() {}

    /** Open one screen of the app: "ask", "needs", "now", "next". */
    static Intent open(Context context, String screen) {
        Intent intent = new Intent(context, LauncherActivity.class);
        intent.setAction(Intent.ACTION_VIEW);
        intent.setData(Uri.parse(context.getString(R.string.console_url) + "#" + screen));
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        return intent;
    }

    /**
     * Open Ask with words already in the box. The words travel in the
     * part of the address after "#", which a browser never sends to a
     * server: they stay on the phone until the owner presses Send.
     */
    static Intent ask(Context context, CharSequence words) {
        String text = words.toString().trim();
        if (text.length() > MAX_TEXT) {
            text = text.substring(0, MAX_TEXT);
        }
        return open(context, "ask/say=" + Uri.encode(text));
    }
}
