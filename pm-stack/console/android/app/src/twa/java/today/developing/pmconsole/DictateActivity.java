package today.developing.pmconsole;

import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.os.Bundle;
import android.speech.RecognizerIntent;
import java.util.ArrayList;

/**
 * Say what you want: the phone's own speech recogniser listens, and the
 * words land in the Ask box. Reached from the app's shortcuts and from
 * the quick-settings tile, so it works with the app closed.
 */
public class DictateActivity extends Activity {
    private static final int REQUEST_SPEECH = 7;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        if (savedInstanceState != null) {
            return; // the recogniser is already on screen
        }
        Intent listen = new Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH);
        listen.putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM);
        listen.putExtra(RecognizerIntent.EXTRA_PROMPT, getString(R.string.dictate_prompt));
        try {
            startActivityForResult(listen, REQUEST_SPEECH);
        } catch (ActivityNotFoundException noRecogniser) {
            // No speech recogniser on this phone: open Ask to type instead.
            startActivity(Links.open(this, "ask"));
            finish();
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQUEST_SPEECH && resultCode == RESULT_OK && data != null) {
            ArrayList<String> heard = data.getStringArrayListExtra(RecognizerIntent.EXTRA_RESULTS);
            if (heard != null && !heard.isEmpty() && !heard.get(0).trim().isEmpty()) {
                startActivity(Links.ask(this, heard.get(0)));
            }
        }
        finish();
    }
}
