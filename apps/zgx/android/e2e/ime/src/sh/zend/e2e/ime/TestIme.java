package sh.zend.e2e.ime;

import android.content.BroadcastReceiver;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.graphics.Color;
import android.inputmethodservice.InputMethodService;
import android.os.SystemClock;
import android.util.Base64;
import android.util.Log;
import android.view.KeyEvent;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.ExtractedText;
import android.view.inputmethod.ExtractedTextRequest;
import android.view.inputmethod.InputConnection;
import java.nio.charset.StandardCharsets;

/**
 * A keyboard the tests drive: `am broadcast -a sh.zend.e2e.ime.CMD -p
 * sh.zend.e2e.ime --es op <op> [--es b64 <text, base64 UTF-8>] [--ei n <n>]
 * --es seq <id>`. Each answer is one log line, `result seq=<id> ...`, with
 * text fields in base64 so umlauts and line breaks survive the log.
 *
 * ops: word (composes the text letter by letter, then commits it, as Gboard
 * types a word), commit, compose, finish, delete (n before the cursor), key
 * (sends key code n down and up through the connection), action (performs
 * editor action n), state (the text around the cursor and the editor's
 * attributes), clip (the clipboard's text), hide.
 */
public class TestIme extends InputMethodService {
    static final String TAG = "ZendE2eIme";
    private BroadcastReceiver receiver;
    private EditorInfo editor;

    static String b64(CharSequence s) {
        if (s == null) return "null";
        return Base64.encodeToString(s.toString().getBytes(StandardCharsets.UTF_8), Base64.NO_WRAP);
    }

    @Override
    public void onCreate() {
        super.onCreate();
        receiver = new BroadcastReceiver() {
            @Override
            public void onReceive(Context context, Intent intent) {
                handle(intent);
            }
        };
        registerReceiver(receiver, new IntentFilter("sh.zend.e2e.ime.CMD"), Context.RECEIVER_EXPORTED);
        Log.i(TAG, "created");
    }

    @Override
    public void onDestroy() {
        unregisterReceiver(receiver);
        super.onDestroy();
    }

    @Override
    public View onCreateInputView() {
        // A plain block where a keyboard would be: the app sees its height
        // as the keyboard's inset.
        View v = new View(this);
        v.setBackgroundColor(Color.rgb(40, 40, 48));
        v.setMinimumHeight((int) (260 * getResources().getDisplayMetrics().density));
        return v;
    }

    @Override
    public boolean onEvaluateFullscreenMode() {
        return false;
    }

    @Override
    public void onStartInput(EditorInfo info, boolean restarting) {
        super.onStartInput(info, restarting);
        editor = info;
        Log.i(TAG, "start inputType=" + info.inputType + " imeOptions=" + info.imeOptions
                + " sel=" + info.initialSelStart + "," + info.initialSelEnd + " restarting=" + restarting
                + " package=" + info.packageName);
    }

    @Override
    public void onStartInputView(EditorInfo info, boolean restarting) {
        super.onStartInputView(info, restarting);
        Log.i(TAG, "shown");
    }

    @Override
    public void onFinishInputView(boolean finishingInput) {
        super.onFinishInputView(finishingInput);
        Log.i(TAG, "hidden");
    }

    @Override
    public void onUpdateSelection(int oldSelStart, int oldSelEnd, int newSelStart, int newSelEnd,
            int candidatesStart, int candidatesEnd) {
        super.onUpdateSelection(oldSelStart, oldSelEnd, newSelStart, newSelEnd, candidatesStart, candidatesEnd);
        Log.i(TAG, "selection " + newSelStart + "," + newSelEnd + " composing " + candidatesStart + "," + candidatesEnd);
    }

    private void handle(Intent intent) {
        String op = intent.getStringExtra("op");
        String seq = intent.getStringExtra("seq");
        String raw = intent.getStringExtra("b64");
        String text = raw == null ? "" : new String(Base64.decode(raw, Base64.DEFAULT), StandardCharsets.UTF_8);
        int n = intent.getIntExtra("n", 0);
        InputConnection ic = getCurrentInputConnection();
        String out = "";
        boolean ok = false;
        try {
            if (ic == null && !"clip".equals(op) && !"hide".equals(op)) {
                out = " error=no-connection";
            } else switch (op == null ? "" : op) {
                case "word": {
                    // As Gboard types a word: each letter grows the
                    // composition, then the word is committed.
                    ok = true;
                    int i = 0;
                    while (i < text.length()) {
                        i += Character.charCount(text.codePointAt(i));
                        ok &= ic.setComposingText(text.substring(0, i), 1);
                        SystemClock.sleep(15);
                    }
                    ok &= ic.commitText(text, 1);
                    break;
                }
                case "commit":
                    ok = ic.commitText(text, 1);
                    break;
                case "compose":
                    ok = ic.setComposingText(text, 1);
                    break;
                case "finish":
                    ok = ic.finishComposingText();
                    break;
                case "delete":
                    ok = ic.deleteSurroundingText(n, 0);
                    break;
                case "key": {
                    long t = SystemClock.uptimeMillis();
                    ok = ic.sendKeyEvent(new KeyEvent(t, t, KeyEvent.ACTION_DOWN, n, 0, 0, -1, 0,
                            KeyEvent.FLAG_SOFT_KEYBOARD | KeyEvent.FLAG_KEEP_TOUCH_MODE));
                    ok &= ic.sendKeyEvent(new KeyEvent(t, SystemClock.uptimeMillis(), KeyEvent.ACTION_UP, n, 0, 0, -1, 0,
                            KeyEvent.FLAG_SOFT_KEYBOARD | KeyEvent.FLAG_KEEP_TOUCH_MODE));
                    break;
                }
                case "action":
                    ok = ic.performEditorAction(n);
                    break;
                case "state": {
                    CharSequence before = ic.getTextBeforeCursor(2000, 0);
                    CharSequence after = ic.getTextAfterCursor(2000, 0);
                    ExtractedText et = ic.getExtractedText(new ExtractedTextRequest(), 0);
                    ok = before != null;
                    out = " before=" + b64(before) + " after=" + b64(after)
                            + " extracted=" + (et == null ? "null" : b64(et.text) + " sel=" + et.selectionStart + "," + et.selectionEnd)
                            + (editor == null ? "" : " inputType=" + editor.inputType + " imeOptions=" + editor.imeOptions);
                    break;
                }
                case "clip": {
                    ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
                    ClipData clip = cm.getPrimaryClip();
                    ok = clip != null && clip.getItemCount() > 0;
                    out = " clip=" + (ok ? b64(clip.getItemAt(0).coerceToText(this)) : "null");
                    break;
                }
                case "setclip": {
                    ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
                    cm.setPrimaryClip(ClipData.newPlainText("e2e", text));
                    ok = true;
                    break;
                }
                case "hide":
                    requestHideSelf(0);
                    ok = true;
                    break;
                default:
                    out = " error=unknown-op";
            }
        } catch (RuntimeException e) {
            out = " error=" + b64(e.toString());
        }
        Log.i(TAG, "result seq=" + seq + " op=" + op + " ok=" + ok
                + " ic=" + (ic == null ? "null" : ic.getClass().getName()) + out);
    }
}
