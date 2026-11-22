package today.developing.pmconsole;

import android.app.PendingIntent;
import android.content.Intent;
import android.os.Build;
import android.service.quicksettings.Tile;
import android.service.quicksettings.TileService;

/** A quick-settings tile: pull down the shade, tap, and say what you want. */
public class AskTileService extends TileService {
    @Override
    public void onStartListening() {
        Tile tile = getQsTile();
        if (tile != null) {
            tile.setState(Tile.STATE_INACTIVE);
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
