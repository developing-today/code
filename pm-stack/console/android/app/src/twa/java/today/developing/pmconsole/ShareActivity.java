package today.developing.pmconsole;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;

/**
 * Receives text from anywhere on the phone: "Share" in another app, or
 * "Ask the fleet" in the menu that appears when text is selected. The
 * text lands in the Ask box, ready to edit and send — unless it is an
 * enroll command, which lands in provisioning on the Agents screen
 * instead (#14377): a bootstrap token pasted into an Ask would file
 * it as a task and leave the new agent unenrolled.
 */
public class ShareActivity extends Activity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        Intent intent = getIntent();
        CharSequence text = null;
        if (Intent.ACTION_SEND.equals(intent.getAction())) {
            text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT);
            String subject = intent.getStringExtra(Intent.EXTRA_SUBJECT);
            if (subject != null && !subject.isEmpty() && (text == null || !text.toString().contains(subject))) {
                text = text == null ? subject : subject + "\n" + text;
            }
        } else if (Intent.ACTION_PROCESS_TEXT.equals(intent.getAction())) {
            text = intent.getCharSequenceExtra(Intent.EXTRA_PROCESS_TEXT);
        }
        if (text != null && text.toString().trim().length() > 0) {
            startActivity(Links.enrollCommand(text) ? Links.provision(this, text) : Links.ask(this, text));
        } else {
            startActivity(Links.open(this, "ask"));
        }
        finish();
    }
}
