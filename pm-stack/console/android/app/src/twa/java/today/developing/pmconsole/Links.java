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

    /**
     * Whether shared text is an enroll command (#14377): a
     * `fleet agent enroll … --bootstrap <token>` line, as the
     * console's provisioning form shows it. The token is opaque — it
     * has no shape of its own — so a command is known by its words:
     * it must name the enroll step and carry a bootstrap token. The
     * console reads the binding out of it the same way.
     */
    static boolean enrollCommand(CharSequence text) {
        if (text == null) {
            return false;
        }
        String shared = text.toString();
        return shared.contains("enroll") && shared.contains("--bootstrap");
    }

    /**
     * Open provisioning on the Agents screen with an enroll command
     * that was shared to the phone. The command travels after the
     * "#" like words for the Ask box do: it stays on the phone, and
     * the console shows it there once, as it shows a mint.
     */
    static Intent provision(Context context, CharSequence words) {
        String text = words.toString().trim();
        if (text.length() > MAX_TEXT) {
            text = text.substring(0, MAX_TEXT);
        }
        return open(context, "more/agents/enroll=" + Uri.encode(text));
    }
}
