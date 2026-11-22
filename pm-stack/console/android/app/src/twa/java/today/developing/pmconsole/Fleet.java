package today.developing.pmconsole;

import android.content.Context;
import android.content.SharedPreferences;
import android.util.Base64;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;
import org.json.JSONObject;

/**
 * What this phone knows of the fleet without opening the app: a short
 * summary (counts, and the top line of each list) read with a token of
 * the phone's own. The token reads that summary and nothing else.
 *
 * <p>Linking passes no secret through an address or another app. The
 * phone makes a random verifier and shows the signed-in owner only its
 * hash. Once the owner approves the hash, the phone presents the
 * verifier and is given its token.
 */
final class Fleet {
    static final int NONE = 0;
    static final int LINKED = 1;
    static final int WAITING = 2;
    static final int FAILED = 3;

    private static final String PREFS = "fleet";
    private static final String KEY_TOKEN = "token";
    private static final String KEY_VERIFIER = "verifier";
    private static final String KEY_VERIFIER_AT = "verifierAt";
    private static final String KEY_SUMMARY = "summary";
    private static final String KEY_FETCHED = "fetchedAt";
    /** A link request the owner has not approved is forgotten after this long. */
    private static final long VERIFIER_MS = 30 * 60 * 1000L;
    private static final int CONNECT_MS = 3000;
    private static final int READ_MS = 5000;
    private static final int MAX_BODY = 64 * 1024;

    private Fleet() {}

    private static SharedPreferences prefs(Context context) {
        return context.getApplicationContext().getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }

    static boolean linked(Context context) {
        return prefs(context).getString(KEY_TOKEN, null) != null;
    }

    /** The verifier of a link that is waiting for approval, or null. */
    static String pendingVerifier(Context context) {
        SharedPreferences prefs = prefs(context);
        String verifier = prefs.getString(KEY_VERIFIER, null);
        if (verifier == null) {
            return null;
        }
        if (System.currentTimeMillis() - prefs.getLong(KEY_VERIFIER_AT, 0) > VERIFIER_MS) {
            prefs.edit().remove(KEY_VERIFIER).remove(KEY_VERIFIER_AT).apply();
            return null;
        }
        return verifier;
    }

    /** Start a link: a new random verifier, kept until it is used or grows old. */
    static String newVerifier(Context context) {
        byte[] random = new byte[32];
        new SecureRandom().nextBytes(random);
        String verifier = base64Url(random);
        prefs(context)
                .edit()
                .putString(KEY_VERIFIER, verifier)
                .putLong(KEY_VERIFIER_AT, System.currentTimeMillis())
                .apply();
        return verifier;
    }

    /** What the owner is shown and approves: the SHA-256 of the verifier. */
    static String challenge(String verifier) throws Exception {
        return base64Url(MessageDigest.getInstance("SHA-256").digest(verifier.getBytes(StandardCharsets.US_ASCII)));
    }

    private static String base64Url(byte[] bytes) {
        return Base64.encodeToString(bytes, Base64.URL_SAFE | Base64.NO_PADDING | Base64.NO_WRAP);
    }

    /** Present the verifier. LINKED once the owner has approved it. Not for the main thread. */
    static int tryLink(Context context) {
        String verifier = pendingVerifier(context);
        if (verifier == null) {
            return NONE;
        }
        try {
            Answer answer =
                    request(context, "POST", "/console/device/link", null, new JSONObject().put("verifier", verifier).toString());
            if (answer.code == 200) {
                String token = new JSONObject(answer.body).getString("token");
                prefs(context).edit().putString(KEY_TOKEN, token).remove(KEY_VERIFIER).remove(KEY_VERIFIER_AT).apply();
                return LINKED;
            }
            return answer.code == 404 ? WAITING : FAILED;
        } catch (Exception failed) {
            return FAILED;
        }
    }

    /** Read the summary again and keep it. True when a fresh one was stored. Not for the main thread. */
    static boolean refresh(Context context) {
        String token = prefs(context).getString(KEY_TOKEN, null);
        if (token == null) {
            return false;
        }
        try {
            Answer answer = request(context, "GET", "/console/device/summary", token, null);
            if (answer.code == 200) {
                new JSONObject(answer.body); // kept only if it reads as a summary
                prefs(context)
                        .edit()
                        .putString(KEY_SUMMARY, answer.body)
                        .putLong(KEY_FETCHED, System.currentTimeMillis())
                        .apply();
                return true;
            }
            // The owner removed this phone: forget the token and what it read.
            if (answer.code == 401 && new JSONObject(answer.body).optBoolean("unlinked", false)) {
                prefs(context).edit().remove(KEY_TOKEN).remove(KEY_SUMMARY).remove(KEY_FETCHED).apply();
            }
            return false;
        } catch (Exception failed) {
            return false;
        }
    }

    /** The last summary read, or null. */
    static JSONObject cached(Context context) {
        String stored = prefs(context).getString(KEY_SUMMARY, null);
        if (stored == null) {
            return null;
        }
        try {
            return new JSONObject(stored);
        } catch (Exception unreadable) {
            return null;
        }
    }

    /** When the last summary was read, in milliseconds, or 0. */
    static long fetchedAt(Context context) {
        return prefs(context).getLong(KEY_FETCHED, 0);
    }

    private static final class Answer {
        final int code;
        final String body;

        Answer(int code, String body) {
            this.code = code;
            this.body = body;
        }
    }

    private static Answer request(Context context, String method, String path, String bearer, String json)
            throws IOException {
        HttpURLConnection connection =
                (HttpURLConnection) new URL(context.getString(R.string.site_url) + path).openConnection();
        try {
            connection.setRequestMethod(method);
            connection.setConnectTimeout(CONNECT_MS);
            connection.setReadTimeout(READ_MS);
            // A redirect would be the sign-in page, which is no answer to this.
            connection.setInstanceFollowRedirects(false);
            connection.setUseCaches(false);
            connection.setRequestProperty("Accept", "application/json");
            if (bearer != null) {
                connection.setRequestProperty("Authorization", "Bearer " + bearer);
            }
            if (json != null) {
                byte[] out = json.getBytes(StandardCharsets.UTF_8);
                connection.setDoOutput(true);
                connection.setRequestProperty("Content-Type", "application/json");
                connection.setFixedLengthStreamingMode(out.length);
                try (OutputStream stream = connection.getOutputStream()) {
                    stream.write(out);
                }
            }
            int code = connection.getResponseCode();
            InputStream stream = code >= 400 ? connection.getErrorStream() : connection.getInputStream();
            return new Answer(code, stream == null ? "" : read(stream));
        } finally {
            connection.disconnect();
        }
    }

    private static String read(InputStream stream) throws IOException {
        try (InputStream in = stream) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buffer = new byte[4096];
            int n;
            while ((n = in.read(buffer)) > 0 && out.size() < MAX_BODY) {
                out.write(buffer, 0, n);
            }
            return new String(out.toByteArray(), StandardCharsets.UTF_8);
        }
    }
}
