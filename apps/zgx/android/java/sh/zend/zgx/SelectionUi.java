package sh.zend.zgx;

import android.content.res.TypedArray;
import android.graphics.Rect;
import android.graphics.drawable.Drawable;
import android.view.ActionMode;
import android.view.Gravity;
import android.view.Menu;
import android.view.MenuItem;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.widget.ImageView;
import android.widget.PopupWindow;

/**
 * A text selection as Android shows one: the theme's two handles under its
 * ends (popup windows of their own, as TextView's are), dragged to move each
 * end, and the floating toolbar above it (Copy, Select all; Cut and Paste in
 * an editor). The app (android_ime.rs) says where the ends are and moves
 * them; the toolbar hides while a handle is held. All on the UI thread.
 */
final class SelectionUi {
    final ZendActivity activity;
    final Handle start, end;
    ActionMode mode;
    final Rect content = new Rect();
    boolean editable;

    SelectionUi(ZendActivity activity) {
        this.activity = activity;
        start = new Handle(0, android.R.attr.textSelectHandleLeft);
        end = new Handle(1, android.R.attr.textSelectHandleRight);
    }

    void show(float sx, float sy, float ex, float ey, float line, boolean editable, boolean dragging) {
        this.editable = editable;
        start.place(sx, sy, line);
        end.place(ex, ey, line);
        int[] at = new int[2];
        activity.input.getLocationInWindow(at);
        content.set((int) Math.min(sx, ex) - at[0], (int) (Math.min(sy, ey) - line) - at[1],
                (int) Math.max(sx, ex) - at[0], (int) Math.max(sy, ey) - at[1]);
        if (content.width() < 1) content.right = content.left + 1;
        if (dragging) {
            if (mode != null) mode.hide(Long.MAX_VALUE);
        } else if (mode == null) {
            mode = activity.input.startActionMode(new Toolbar(), ActionMode.TYPE_FLOATING);
        } else {
            mode.invalidate();
            mode.invalidateContentRect();
            mode.hide(0);
        }
    }

    void hide() {
        start.hide();
        end.hide();
        if (mode != null) {
            ActionMode m = mode;
            mode = null;
            m.finish();
        }
    }

    final class Toolbar extends ActionMode.Callback2 {
        @Override
        public boolean onCreateActionMode(ActionMode m, Menu menu) {
            return true;
        }

        @Override
        public boolean onPrepareActionMode(ActionMode m, Menu menu) {
            menu.clear();
            if (editable) menu.add(Menu.NONE, 1, 0, android.R.string.cut);
            menu.add(Menu.NONE, 0, 1, android.R.string.copy);
            if (editable) menu.add(Menu.NONE, 2, 2, android.R.string.paste);
            menu.add(Menu.NONE, 3, 3, android.R.string.selectAll);
            for (int i = 0; i < menu.size(); i++) menu.getItem(i).setShowAsAction(MenuItem.SHOW_AS_ACTION_ALWAYS);
            return true;
        }

        @Override
        public boolean onActionItemClicked(ActionMode m, MenuItem item) {
            ZendActivity.nativeMenu(item.getItemId());
            return true;
        }

        @Override
        public void onDestroyActionMode(ActionMode m) {
            if (mode == m) mode = null;
        }

        @Override
        public void onGetContentRect(ActionMode m, View view, Rect out) {
            out.set(content);
        }
    }

    final class Handle {
        final int which;
        final PopupWindow popup;
        final ImageView image;
        final float hotspot;
        float line;
        /** From the finger to where the end goes, while dragged. */
        float dx, dy;

        Handle(int which, int attr) {
            this.which = which;
            TypedArray a = activity.getTheme().obtainStyledAttributes(new int[] {attr});
            Drawable d = a.getDrawable(0);
            a.recycle();
            image = new ImageView(activity);
            image.setImageDrawable(d);
            int w = d == null ? 48 : d.getIntrinsicWidth(), h = d == null ? 48 : d.getIntrinsicHeight();
            // Where the end is on the handle: TextView's hotspots.
            hotspot = which == 0 ? w * 3f / 4f : w / 4f;
            popup = new PopupWindow(image, w, h, false);
            popup.setBackgroundDrawable(null);
            popup.setClippingEnabled(false);
            popup.setSplitTouchEnabled(true);
            popup.setTouchable(true);
            image.setOnTouchListener((v, e) -> touch(e));
        }

        void place(float x, float y, float line) {
            this.line = line;
            int px = Math.round(x - hotspot), py = Math.round(y);
            if (popup.isShowing()) popup.update(px, py, -1, -1);
            else popup.showAtLocation(activity.input, Gravity.NO_GRAVITY, px, py);
        }

        void hide() {
            if (popup.isShowing()) popup.dismiss();
        }

        boolean touch(MotionEvent e) {
            int[] at = new int[2];
            image.getLocationOnScreen(at);
            switch (e.getActionMasked()) {
                case MotionEvent.ACTION_DOWN:
                    // The end sits at the hotspot, its line's middle above the handle's top.
                    dx = at[0] + hotspot - e.getRawX();
                    dy = at[1] - line / 2f - e.getRawY();
                    ZendActivity.nativeHandle(which, e.getRawX() + dx, e.getRawY() + dy, 0);
                    return true;
                case MotionEvent.ACTION_MOVE:
                    ZendActivity.nativeHandle(which, e.getRawX() + dx, e.getRawY() + dy, 1);
                    return true;
                case MotionEvent.ACTION_UP:
                case MotionEvent.ACTION_CANCEL:
                    ZendActivity.nativeHandle(which, e.getRawX() + dx, e.getRawY() + dy, 2);
                    return true;
                default:
                    return false;
            }
        }
    }
}
