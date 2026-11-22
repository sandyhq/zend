package sh.zend.zgx;

import android.app.NativeActivity;
import android.content.Context;
import android.os.Bundle;
import android.util.Log;
import android.text.Editable;
import android.text.Selection;
import android.view.HapticFeedbackConstants;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;

/**
 * Zend's activity: NativeActivity (the app is libzend_android.so, run by
 * android_main) with what only a View has: the keyboard's text connection
 * (InputConnection), the selection's handles and floating toolbar
 * (SelectionUi), haptics. zgx-desktop's android_ime.rs is the other half,
 * through the native methods below.
 *
 * The keyboard edits a mirror of the focused editor's text (BaseInputConnection
 * applies Android's own rules: composing regions, cursor positions, surrogate
 * pairs); each edit goes to the app as one delta against the mirror as it
 * was. The app answers with its editor's state, tagged with the last delta it
 * applied: an answer older than the keyboard's latest edit is dropped (the
 * next one follows), so no keystroke is lost to a crossing update.
 */
public class ZendActivity extends NativeActivity {
    static {
        System.loadLibrary("c++_shared");
        System.loadLibrary("zend_android");
    }

    // ---- to the app (android_ime.rs); called on the UI thread

    /** The mirror's [start, oldEnd) became text; then its selection and composing region (-1: none). */
    static native void nativeEdit(int seq, int start, int oldEnd, String text, int selStart, int selEnd,
            int compStart, int compEnd);
    /** A selection handle (0 start, 1 end) dragged to x, y (window pixels); phase 0 down, 1 move, 2 up. */
    static native void nativeHandle(int which, float x, float y, int phase);
    /** A toolbar item chosen: 0 copy, 1 cut, 2 paste, 3 select all. */
    static native void nativeMenu(int item);
    /** The activity is here (on its UI thread): the app keeps it to call back. */
    static native void nativeAttach(ZendActivity activity);

    InputView input;
    SelectionUi selection;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        input = new InputView(this);
        // A view that never draws and takes no touches (NativeActivity's input
        // queue has them): it holds focus for the keyboard, and the selection's
        // floating toolbar is placed in it, which shows only over its view's
        // bounds, so it spans the window.
        addContentView(input, new ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT));
        selection = new SelectionUi(this);
        nativeAttach(this);
    }

    @Override
    protected void onDestroy() {
        if (selection != null) selection.hide();
        super.onDestroy();
    }

    // ---- from the app (any thread)

    /** An editor took focus: the keyboard for it, its text and selection. */
    public void imeStart(int inputType, int imeOptions, String text, int selStart, int selEnd, boolean show) {
        runOnUiThread(() -> input.start(inputType, imeOptions, text, selStart, selEnd, show));
    }

    /** No editor takes text: the keyboard down, the connection closed. */
    public void imeStop() {
        runOnUiThread(() -> input.stop());
    }

    /** The keyboard up again for the editor that has it (a tap on it). */
    public void imeShow() {
        runOnUiThread(() -> input.show());
    }

    /** The editor's state, after the app's own edits or the keyboard's deltas up to applied. */
    public void imeState(int applied, String text, int selStart, int selEnd, int compStart, int compEnd) {
        runOnUiThread(() -> input.state(applied, text, selStart, selEnd, compStart, compEnd));
    }

    /** A long press took hold: the haptic tick Android gives. */
    public void held() {
        runOnUiThread(() -> getWindow().getDecorView().performHapticFeedback(HapticFeedbackConstants.LONG_PRESS));
    }

    /**
     * The text selection's ends (window pixels: each end's bottom, and its line's height), or none
     * (shown false). editable: Cut and Paste too; dragging: a handle is held (no toolbar meanwhile).
     */
    public void selection(boolean shown, float sx, float sy, float ex, float ey, float line, boolean editable,
            boolean dragging) {
        runOnUiThread(() -> {
            if (shown) selection.show(sx, sy, ex, ey, line, editable, dragging);
            else selection.hide();
        });
    }

    // ---- the keyboard's view

    static final class InputView extends View {
        final InputMethodManager imm;
        boolean editing;
        int inputType = EditorInfo.TYPE_CLASS_TEXT;
        int imeOptions;
        final Editable mirror = Editable.Factory.getInstance().newEditable("");
        /** The last delta sent; the batch depth; the connection the keyboard has. */
        int sent, batch;
        Conn conn;

        InputView(Context context) {
            super(context);
            imm = (InputMethodManager) context.getSystemService(Context.INPUT_METHOD_SERVICE);
            setFocusable(true);
            setFocusableInTouchMode(true);
        }

        @Override
        public boolean onCheckIsTextEditor() {
            return editing;
        }

        @Override
        public InputConnection onCreateInputConnection(EditorInfo out) {
            if (!editing) return null;
            out.inputType = inputType;
            out.imeOptions = imeOptions;
            out.initialSelStart = Selection.getSelectionStart(mirror);
            out.initialSelEnd = Selection.getSelectionEnd(mirror);
            if (android.os.Build.VERSION.SDK_INT >= 30) out.setInitialSurroundingText(mirror);
            conn = new Conn(this);
            return conn;
        }

        void start(int type, int options, String text, int s, int e, boolean show) {
            Log.i("ZendIme", "start inputType=0x" + Integer.toHexString(type) + " imeOptions=0x"
                    + Integer.toHexString(options) + " show=" + show);
            editing = true;
            inputType = type;
            imeOptions = options;
            replaceMirror(text, s, e, -1, -1);
            if (!hasFocus()) requestFocus();
            imm.restartInput(this);
            if (show) imm.showSoftInput(this, 0);
        }

        void show() {
            if (!editing) return;
            if (!hasFocus()) requestFocus();
            imm.showSoftInput(this, 0);
        }

        void stop() {
            if (!editing) return;
            Log.i("ZendIme", "stop");
            editing = false;
            imm.hideSoftInputFromWindow(getWindowToken(), 0);
            imm.restartInput(this);
        }

        void state(int answered, String text, int s, int e, int cs, int ce) {
            // An answer from before the keyboard's last edit: the next one has it.
            if (!editing || answered < sent || batch > 0) return;
            boolean sameText = mirror.toString().equals(text);
            int[] comp = composing();
            if (sameText && Selection.getSelectionStart(mirror) == s && Selection.getSelectionEnd(mirror) == e
                    && comp[0] == cs && comp[1] == ce) return;
            replaceMirror(text, s, e, cs, ce);
            if (!sameText) {
                // The app changed the text (sent, cleared, typed by a key): the keyboard
                // reads it again.
                imm.restartInput(this);
            } else {
                imm.updateSelection(this, Selection.getSelectionStart(mirror), Selection.getSelectionEnd(mirror),
                        composing()[0], composing()[1]);
            }
        }

        int[] composing() {
            return new int[] {BaseInputConnection.getComposingSpanStart(mirror),
                    BaseInputConnection.getComposingSpanEnd(mirror)};
        }

        void replaceMirror(String text, int s, int e, int cs, int ce) {
            BaseInputConnection.removeComposingSpans(mirror);
            mirror.replace(0, mirror.length(), text);
            int n = mirror.length();
            Selection.setSelection(mirror, clamp(s, n), clamp(e, n));
            // (The composing span is the connection's to set; without one no keyboard reads it.)
            if (cs >= 0 && ce > cs && ce <= n && conn != null) conn.region(cs, ce);
        }

        static int clamp(int v, int n) {
            return Math.max(0, Math.min(v, n));
        }

        /** After an edit of the keyboard's: the delta to the app, the selection to the keyboard. */
        void edited(String before) {
            String after = mirror.toString();
            int a = 0, max = Math.min(before.length(), after.length());
            while (a < max && before.charAt(a) == after.charAt(a)) a++;
            int be = before.length(), ae = after.length();
            while (be > a && ae > a && before.charAt(be - 1) == after.charAt(ae - 1)) {
                be--;
                ae--;
            }
            // Never split a surrogate pair.
            if (a > 0 && Character.isHighSurrogate(before.charAt(a - 1))) a--;
            if (ae < after.length() && be < before.length() && Character.isLowSurrogate(after.charAt(ae))) {
                ae++;
                be++;
            }
            int s = Selection.getSelectionStart(mirror), e = Selection.getSelectionEnd(mirror);
            int[] comp = composing();
            nativeEdit(++sent, a, be, after.substring(a, ae), s, e, comp[0], comp[1]);
            imm.updateSelection(this, s, e, comp[0], comp[1]);
        }
    }

    static final class Conn extends BaseInputConnection {
        final InputView view;
        String before;

        Conn(InputView view) {
            super(view, true);
            this.view = view;
        }

        @Override
        public Editable getEditable() {
            return view.mirror;
        }

        /** Setting the app's composing region: not the keyboard's edit, nothing to send. */
        boolean quiet;

        void region(int start, int end) {
            quiet = true;
            try {
                super.setComposingRegion(start, end);
            } finally {
                quiet = false;
            }
        }

        @Override
        public boolean beginBatchEdit() {
            if (quiet) return true;
            if (view.batch++ == 0) before = view.mirror.toString();
            return true;
        }

        @Override
        public boolean endBatchEdit() {
            if (quiet) return false;
            if (view.batch > 0 && --view.batch == 0 && before != null) {
                view.edited(before);
                before = null;
            }
            return view.batch > 0;
        }

        private boolean edit(java.util.function.BooleanSupplier op) {
            beginBatchEdit();
            boolean ok = op.getAsBoolean();
            endBatchEdit();
            return ok;
        }

        @Override
        public boolean commitText(CharSequence text, int pos) {
            return edit(() -> super.commitText(text, pos));
        }

        @Override
        public boolean setComposingText(CharSequence text, int pos) {
            return edit(() -> super.setComposingText(text, pos));
        }

        @Override
        public boolean setComposingRegion(int start, int end) {
            return edit(() -> super.setComposingRegion(start, end));
        }

        @Override
        public boolean finishComposingText() {
            return edit(() -> super.finishComposingText());
        }

        @Override
        public boolean deleteSurroundingText(int before, int after) {
            return edit(() -> super.deleteSurroundingText(before, after));
        }

        @Override
        public boolean deleteSurroundingTextInCodePoints(int before, int after) {
            return edit(() -> super.deleteSurroundingTextInCodePoints(before, after));
        }

        @Override
        public boolean setSelection(int start, int end) {
            return edit(() -> super.setSelection(start, end));
        }

        @Override
        public boolean performEditorAction(int action) {
            // A field's action key (Done, Search, Send): its Enter, which submits it.
            long now = android.os.SystemClock.uptimeMillis();
            super.sendKeyEvent(new KeyEvent(now, now, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ENTER, 0, 0,
                    -1, 0, KeyEvent.FLAG_SOFT_KEYBOARD | KeyEvent.FLAG_EDITOR_ACTION));
            super.sendKeyEvent(new KeyEvent(now, now, KeyEvent.ACTION_UP, KeyEvent.KEYCODE_ENTER, 0, 0,
                    -1, 0, KeyEvent.FLAG_SOFT_KEYBOARD | KeyEvent.FLAG_EDITOR_ACTION));
            return true;
        }

        @Override
        public boolean sendKeyEvent(KeyEvent event) {
            // A text area's Enter and Backspace from the keyboard edit its text, as a
            // TextView's do; a field's Enter goes to the app as a key (it submits),
            // as do Backspace with nothing before the caret (the composer's
            // attachments) and every other key.
            int code = event.getKeyCode();
            boolean multiline = (view.inputType & EditorInfo.TYPE_TEXT_FLAG_MULTI_LINE) != 0;
            Editable m = view.mirror;
            int s = Selection.getSelectionStart(m), e = Selection.getSelectionEnd(m);
            boolean plain = (event.getMetaState() & ~KeyEvent.META_SHIFT_MASK) == 0;
            if (plain && code == KeyEvent.KEYCODE_ENTER && multiline) {
                if (event.getAction() == KeyEvent.ACTION_DOWN) commitText("\n", 1);
                return true;
            }
            if (event.getMetaState() == 0 && code == KeyEvent.KEYCODE_DEL && (s != e || s > 0)) {
                if (event.getAction() == KeyEvent.ACTION_DOWN) {
                    if (s != e) commitText("", 1);
                    else deleteSurroundingTextInCodePoints(1, 0);
                }
                return true;
            }
            return super.sendKeyEvent(event);
        }

        @Override
        public boolean performContextMenuAction(int id) {
            if (id == android.R.id.copy) nativeMenu(0);
            else if (id == android.R.id.cut) nativeMenu(1);
            else if (id == android.R.id.paste) nativeMenu(2);
            else if (id == android.R.id.selectAll) nativeMenu(3);
            else return false;
            return true;
        }
    }
}
