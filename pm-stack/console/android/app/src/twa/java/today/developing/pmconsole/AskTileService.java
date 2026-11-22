package today.developing.pmconsole;

import android.app.PendingIntent;
import android.content.Intent;
import android.os.Build;
import android.service.quicksettings.Tile;
import android.service.quicksettings.TileService;
import org.json.JSONObject;

/**
 * A quick-settings tile: pull down the shade, tap, and say what you want.
 * On a linked phone it also shows how many things need the owner.
 */
public class AskTileService extends TileService {
    @Override
    public void onStartListening() {
        Tile tile = getQsTile();
        if (tile != null) {
            tile.setState(Tile.STATE_INACTIVE);
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                // Under the label, how many things are waiting, when this phone knows.
                JSONObject summary = Fleet.cached(this);
                JSONObject needs = summary == null ? null : summary.optJSONObject("needs");
                int n = needs == null ? 0 : needs.optInt("total", 0);
                tile.setSubtitle(n > 0 ? getResources().getQuantityString(R.plurals.tile_needs, n, n) : null);
            }
            tile.updateTile();
        }
    }

    @Override
    @SuppressWarnings("deprecation")
    public void onClick() {
        Intent dictate = new Intent(this, DictateActivity.class);
        dictate.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startActivityAndCollapse(
                    PendingIntent.getActivity(
                            this, 0, dictate, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT));
        } else {
            startActivityAndCollapse(dictate);
        }
    }
}
