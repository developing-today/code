package today.developing.pmconsole;

import android.app.Activity;
import android.os.Bundle;
import java.util.Locale;

/**
 * Opens one screen of the app from a long press on its icon. It runs in
 * a short-lived task of its own, so the app that is already open is
 * brought forward and moved to that screen instead of being thrown away
 * and loaded again (a launcher clears the task a shortcut starts in).
 */
public class OpenActivity extends Activity {
    private static final String PREFIX = "today.developing.pmconsole.OPEN_";

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        startActivity(Links.open(this, screen(getIntent().getAction())));
        finish();
    }

    /** The screen an action names; anything unknown opens Now.
     * Messages is the console's `#more/messages` (#14377): the name
     * the shortcut carries is not the route, so it is translated. */
    static String screen(String action) {
        if (action != null && action.startsWith(PREFIX)) {
            String name = action.substring(PREFIX.length()).toLowerCase(Locale.ROOT);
            if (name.equals("ask") || name.equals("needs") || name.equals("next")) {
                return name;
            }
            if (name.equals("messages")) {
                return "more/messages";
            }
        }
        return "now";
    }
}
