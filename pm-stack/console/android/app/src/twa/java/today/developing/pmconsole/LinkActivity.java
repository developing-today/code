package today.developing.pmconsole;

import android.app.Activity;
import android.content.Context;
import android.net.Uri;
import android.os.Bundle;
import android.widget.Toast;

/**
 * Links this phone so its home-screen widget can read the summary.
 *
 * <p>Started from the widget, it opens the signed-in app at a screen that
 * asks the owner to approve this phone, showing only a hash. Started
 * again by that screen once the owner has approved (the address
 * pmconsole://finish, which carries nothing), it collects the phone's
 * token and fills the widget.
 */
public class LinkActivity extends Activity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        Uri data = getIntent().getData();
        boolean approved = data != null && "pmconsole".equals(data.getScheme());
        if (approved || Fleet.linked(this) || Fleet.pendingVerifier(this) != null) {
            // A request may already be approved: collect first, ask only if it is not.
            collect(!approved);
            return;
        }
        ask(Fleet.newVerifier(this));
        StatusWidget.draw(this);
        finish();
    }

    /** Open the signed-in app at the screen that approves this phone. */
    private void ask(String verifier) {
        try {
            startActivity(Links.open(this, "more/link=" + Fleet.challenge(verifier)));
        } catch (Exception failed) {
            Toast.makeText(this, R.string.link_failed, Toast.LENGTH_LONG).show();
        }
    }

    /** Present the verifier and, once linked, read the first summary. */
    private void collect(final boolean askWhenWaiting) {
        final Context app = getApplicationContext();
        new Thread(
                        () -> {
                            final int result = Fleet.linked(app) ? Fleet.LINKED : Fleet.tryLink(app);
                            if (result == Fleet.LINKED) {
                                Fleet.refresh(app);
                            }
                            StatusWidget.draw(app);
                            runOnUiThread(
                                    () -> {
                                        String waiting = Fleet.pendingVerifier(app);
                                        if (result == Fleet.LINKED) {
                                            Toast.makeText(app, R.string.link_done, Toast.LENGTH_LONG).show();
                                        } else if (result == Fleet.WAITING && askWhenWaiting && waiting != null) {
                                            ask(waiting);
                                        } else if (result == Fleet.NONE && askWhenWaiting) {
                                            ask(Fleet.newVerifier(app));
                                            StatusWidget.draw(app);
                                        } else {
                                            Toast.makeText(
                                                            app,
                                                            result == Fleet.FAILED ? R.string.link_failed : R.string.link_waiting,
                                                            Toast.LENGTH_LONG)
                                                    .show();
                                        }
                                        finish();
                                    });
                        },
                        "pm-link")
                .start();
    }
}
