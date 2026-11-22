// Zend's app on an emulator, driven as a person drives a phone: real touches
// (MotionEvents through the input system), text through a real keyboard's
// InputConnection (ime/), and asserts on what the app shows (devtools), what
// the system shows (dumpsys, uiautomator), the clipboard and what reached
// the model. run.sh boots the emulator and calls this.
//
//   node suite.mjs [--apk zend.apk] [--only name,name] [--keep]
import fs from "node:fs";
import path from "node:path";
import * as L from "./lib.mjs";
import * as S from "./stage.mjs";

const args = process.argv.slice(2);
const opt = (k) => { const i = args.indexOf(k); return i >= 0 ? args[i + 1] : null; };
const apk = opt("--apk");
const only = opt("--only")?.split(",");
const keep = args.includes("--keep");
const out = process.env.E2E_OUT || process.env.HOME + "/.cache/android-app/e2e/out";
fs.mkdirSync(out, { recursive: true });
process.env.E2E_OUT = out;

// ---- helpers over the app

const until = async (what, fn, ms = 8000, every = 200) => {
  const t = Date.now();
  let last;
  while (Date.now() - t < ms) {
    try { last = await fn(); if (last) return last; } catch (e) { last = e.message; }
    await L.sleep(every);
  }
  throw new Error(`timed out waiting for ${what}${last ? ` (last: ${String(last).slice(0, 300)})` : ""}`);
};
const first = (target) => L.find(target)[0];
/** A node once it stands still (the same place twice, 300 ms apart). */
const still = async (target) => {
  let was = null;
  return until(`${target}, still`, async () => {
    const n = first(target);
    const same = n && was && n.x === was.x && n.y === was.y;
    was = n;
    if (!same) await L.sleep(300);
    return same && n;
  });
};
const composer = () => first('role:textbox "Message"');
/** The composer's text, as its editor holds it. */
// (Devtools prints a line break as "⏎": zgx-script driver.rs.)
const composerText = () => {
  const n = L.node('role:textbox "Message"');
  const v = n?.value ?? n?.text ?? null;
  return v == null ? null : v.replaceAll("⏎", "\n");
};
// A drag under way, by what shows it (false for none): a chat's (a
// sidebar row's or a tab's, `split.dragging`) or a tab's lifted copy.
const dragging = () =>
  (L.state("split.dragging") === true && "split.dragging") ||
  ["#session-tab-lifted"].find((id) => L.find(id).some((n) => n.w > 0 && !/ hidden/.test(n.line))) ||
  false;

async function sidebar(open) {
  const shown = sidebarShown;
  // Opened by its button; closed by Back, as a person closes a drawer.
  if (shown() !== open) {
    if (open) L.tap(...L.mid(first('role:button "Toggle sidebar"')));
    else L.sh("input keyevent 4");
  }
  await until(`sidebar ${open ? "open" : "closed"}`, () => shown() === open);
  await L.sleep(400);
}
/** The sidebar's chat rows on screen (its buttons, not a tab of the same title behind it). */
function sidebarRows() {
  const bar = L.find("#sidebar").find((n) => n.w > 0 && !/ hidden/.test(n.line));
  if (!bar) return [];
  return L.find("role:button").filter((n) => n.w > 0 && !/ hidden/.test(n.line) && n.x >= bar.x && n.x < bar.x + bar.w && n.y >= bar.y && n.y < bar.y + bar.h && /"E2E /.test(n.what));
}
function sidebarBottom() {
  const bar = L.find("#sidebar").find((n) => n.w > 0 && !/ hidden/.test(n.line));
  return bar ? bar.y + bar.h : 0;
}
/** Whether the sidebar shows (closed, its nodes are listed "hidden"). */
function sidebarShown() {
  return L.find("#sidebar").some((n) => n.w > 0 && !/ hidden/.test(n.line));
}
/** The chat open in the page: its tab's title in the top bar. */
const openTitle = () => L.find("role:tab").find((n) => n.w > 0 && n.y < 120 && !/ hidden/.test(n.line))?.what.match(/"(.*)"/)?.[1] ?? null;
async function openChat(title) {
  // (The list may still be reordering as the engine's chats report in: a
  // row is tapped where it stands still, and the open chat checked.)
  for (let attempt = 0; ; attempt++) {
    await sidebar(true);
    const find = () => sidebarRows().find((n) => n.what === `button "${title}"`);
    // (Rows the list holds beyond its foot aren't in the tree: the list is
    // scrolled on to them, then back up, the chats' order being the engine's.)
    await until("sidebar rows", () => sidebarRows().length > 0);
    for (let i = 0; i < 16 && !find(); i++) {
      const [x, foot] = [L.px(100), sidebarBottom()];
      const [a, b] = i < 6 ? [foot - 60, foot - 260] : [foot - 260, foot - 60];
      L.swipe(x, L.px(a), x, L.px(b), 400);
      await L.sleep(700);
    }
    let row = await until(`row ${title}`, find);
    for (let i = 0; i < 10; i++) {
      await L.sleep(300);
      const again = find();
      if (again && again.y === row.y) break;
      row = again ?? row;
    }
    // A row scrolled past the list's foot is clicked through the inspector:
    // getting there isn't what's under test.
    if (row.y + row.h <= sidebarBottom()) L.tap(...L.mid(row));
    else L.inspect("click", { target: `@${row.id}` });
    await L.sleep(800);
    if (sidebarShown()) await sidebar(false).catch(() => {});
    await until(`the composer of ${title}`, () => composer());
    const open = openTitle();
    if (open === title || open == null) return;
    if (attempt >= 2) throw new Error(`opening ${title} opened ${open}`);
  }
}
async function focusComposer() {
  L.tap(...L.mid(composer()));
  await until("the keyboard", () => L.imeShown(), 6000);
  await until("the keyboard's connection", async () => (await L.ime("state")).ic !== "null");
}
async function hideKeyboard() {
  if (L.imeShown()) { await L.ime("hide"); await until("the keyboard hidden", () => !L.imeShown(), 4000); }
}
/** The app's popup windows (the selection's handles, its toolbar). */
function popups() {
  const dump = L.sh("dumpsys window windows");
  return [...dump.matchAll(/Window\{[0-9a-f]+ u\d+ (PopupWindow:[0-9a-f]+|sh\.zend\.zgx[^}]*)\}/g)].map((m) => m[1]).filter((n) => n.startsWith("PopupWindow"));
}
/** uiautomator's view of every window: [{ text, desc, x, y, w, h }] (pixels). */
function uiNodes() {
  L.sh("uiautomator dump --windows /sdcard/ui.xml >/dev/null 2>&1 || uiautomator dump /sdcard/ui.xml >/dev/null", { ok: true });
  const xml = L.sh("cat /sdcard/ui.xml", { ok: true });
  return [...xml.matchAll(/<node [^>]*>/g)].map((m) => {
    const a = (k) => m[0].match(new RegExp(`${k}="([^"]*)"`))?.[1] ?? "";
    const b = a("bounds").match(/\[(\d+),(\d+)\]\[(\d+),(\d+)\]/);
    return { text: a("text"), desc: a("content-desc"), cls: a("class"), pkg: a("package"), x: +b?.[1], y: +b?.[2], w: b ? b[3] - b[1] : 0, h: b ? b[4] - b[2] : 0 };
  });
}

// ---- the cases

const cases = {
  async umlauts(e) {
    await openChat("E2E row 30");
    await focusComposer();
    for (const [op, text] of [["word", "Grüße"], ["commit", " "], ["word", "äöü"], ["commit", " "], ["word", "ß"]]) await L.ime(op, { text });
    await until(`the composer holding "Grüße äöü ß"`, () => composerText() === "Grüße äöü ß", 4000);
    // (Where Send stands once the composer is still: the keyboard's opening
    // moves it.)
    const send = await still('role:button "Send message"');
    L.tap(...L.mid(send));
    const got = await until('the engine receiving the message', () => e.requests().find((r) => r.text.includes("Gr")), 20000);
    L.eq(got.text, "Grüße äöü ß", "what the engine received");
  },

  async enter(e) {
    await openChat("E2E row 29");
    await focusComposer();
    const st = await L.ime("state");
    // A text area: the keyboard's Enter breaks the line (no action key).
    L.expect((+st.inputType & 0x2000f) === 0x20001, `the composer's inputType ${st.inputType} isn't multi-line text (0x20001)`);
    L.expect((+st.imeOptions & 0x40000000) !== 0, `the composer's imeOptions ${st.imeOptions} lack IME_FLAG_NO_ENTER_ACTION`);
    await L.ime("word", { text: "eins" });
    await L.ime("key", { n: 66 }); // KEYCODE_ENTER, as Gboard sends it in a text area
    await L.ime("word", { text: "zwei" });
    await until('the composer holding "eins\\nzwei"', () => composerText() === "eins\nzwei", 4000).catch((err) => {
      throw new Error(`${err.message} (it holds ${JSON.stringify(composerText())})`);
    });
    await L.sleep(1500);
    L.expect(!e.requests().some((r) => r.text.includes("eins")), "Enter sent the message");
    L.expect(first('role:button "Send message"'), "the send button is gone");
    const send = await still('role:button "Send message"');
    // Sent with its button, the engine gets both lines.
    L.tap(...L.mid(send));
    await until("the engine receiving the two lines", () => e.requests().some((r) => r.text === "eins\nzwei"), 20000).catch((err) => {
      throw new Error(`${err.message} (it got ${JSON.stringify(e.requests().map((r) => r.text).slice(-2))})`);
    });
  },

  async scrollHidesKeyboardOnce() {
    await openChat("E2E long reply");
    await focusComposer();
    const [x, y1, y2] = [L.px(200), L.px(250), L.px(600)];
    const seen = await L.watchIme(async () => {
      for (let i = 0; i < 3; i++) { L.swipe(x, y1, x, y2, 250); await L.sleep(300); L.swipe(x, y2, x, y1, 250); await L.sleep(300); }
    });
    L.expect(seen[0] === true, "the keyboard wasn't up when scrolling began");
    L.expect(seen.includes(false), `scrolling didn't hide the keyboard (${seen.map(Number).join("")})`);
    L.eq(L.reshows(seen), 0, `times it came back (${seen.map(Number).join("")})`);
  },

  async jumpToLatestDoesntFocus() {
    await openChat("E2E long reply");
    await hideKeyboard();
    const [x, y1, y2] = [L.px(200), L.px(250), L.px(700)];
    for (let i = 0; i < 4; i++) { L.swipe(x, y1, x, y2, 200); await L.sleep(250); }
    const jump = await until("the jump-to-latest button", () => first('role:button "Jump to latest"'));
    const seen = await L.watchIme(async () => { L.tap(...L.mid(jump)); });
    L.eq(L.reshows(seen) + (seen[0] ? 1 : 0), 0, `the keyboard showing after the jump (${seen.map(Number).join("")})`);
  },

  async fileViewerSwipe() {
    await hideKeyboard();
    await openFile("README.md");
    const [x, y1, y2] = [L.px(200), L.px(300), L.px(600)];
    const seen = await L.watchIme(async () => {
      for (let i = 0; i < 3; i++) { L.swipe(x, y2, x, y1, 250); await L.sleep(250); L.swipe(x - L.px(120), y1, x + L.px(120), y1, 250); await L.sleep(250); L.swipe(x, y1, x, y2, 250); await L.sleep(250); }
    });
    L.expect(!seen.includes(true), `the keyboard showed (${seen.map(Number).join("")})`);
  },

  async selection() {
    await openChat("E2E long reply");
    await hideKeyboard();
    const para = await until("a paragraph of the reply on screen", () => paragraphs().find((n) => n.y > 150 && n.y + 40 < 700));
    const [px, py] = [L.px(para.x + 40), L.px(para.y + 8)];
    L.down(px, py);
    await L.sleep(900);
    L.up(px, py);
    await L.sleep(700);
    const handles = await until("the selection's handles", () => { const p = popups(); return p.length >= 2 && p; }, 3000);
    // The floating toolbar's Copy, after the end handle is dragged on.
    const frames = handleFrames();
    L.expect(frames.length >= 2, `handle windows ${JSON.stringify(handles)} have no frames`);
    const right = frames.sort((a, b) => a.x - b.x || a.y - b.y).at(-1);
    await L.holdThenDrag(right.x + right.w / 2, right.y + right.h / 3, right.x + right.w / 2 + L.px(120), right.y + right.h / 3, 80, 10);
    await L.sleep(600);
    const copy = await until("the toolbar's Copy", () => uiNodes().find((n) => /^(copy|kopieren)$/i.test(n.text || n.desc)), 4000);
    L.tap(copy.x + copy.w / 2, copy.y + copy.h / 2);
    await L.sleep(500);
    const clip = await L.ime("clip");
    L.expect(clip.ok && clip.clip, "nothing on the clipboard");
    L.expect(clip.clip.trim().split(/\s+/).length >= 2, `the clipboard holds ${JSON.stringify(clip.clip)}: the handle didn't extend the word`);
  },

  async holdDrag(e) {
    await sidebar(true);
    const rows = () => sidebarRows().filter((n) => /E2E row/.test(n.what));
    // Held, then moved: the row drags, and drops into Pinned (first, the
    // list at its top, where Pinned's zone opens, and still).
    for (let i = 0; i < 8; i++) {
      const top = rows()[0]?.y;
      L.swipe(L.px(100), L.px(250), L.px(100), L.px(650), 300);
      await L.sleep(900);
      if (rows()[0]?.y === top) break;
    }
    const row = await until("a sidebar row to hold", () => rows()[1]);
    const title = row.what.match(/"(.*)"/)?.[1];
    const [rx, ry] = L.mid(row);
    L.down(rx, ry);
    await L.sleep(650);
    for (let i = 1; i <= 6; i++) L.move(rx + L.px(4 * i), ry - L.px(10 * i));
    const held = await until("a drag after the hold", () => dragging(), 2000).catch(() => false);
    // (Pinned's drop zone, under its title as a row is dragged: where it
    // stands once its room has opened.)
    await L.sleep(800);
    const zone = first('"Drop here to pin"');
    if (zone) {
      // There as a finger goes, in steps, resting over the zone.
      const [zx, zy] = L.mid(zone), [fx, fy] = [rx + L.px(24), ry - L.px(60)];
      for (let i = 1; i <= 8; i++) { L.move(Math.round(fx + ((zx - fx) * i) / 8), Math.round(fy + ((zy - fy) * i) / 8)); await L.sleep(60); }
    }
    await L.sleep(500);
    L.up(...(zone ? L.mid(zone) : [rx, ry - L.px(60)]));
    await L.sleep(600);
    L.expect(held, "holding a row, then moving, didn't drag it");
    L.expect(zone, "no Pinned drop zone showed while dragging");
    // Pinned: under the list's "Pinned" title, first of the rows.
    await sidebar(true);
    const pin = first('"Pinned"');
    const shown = sidebarRows().filter((n) => /E2E row/.test(n.what)).sort((a, b) => a.y - b.y);
    const at = shown.find((n) => n.what.includes(`"${title}"`));
    L.expect(pin && at && at.y > pin.y && shown[0] === at, `the dropped row ${title} isn't pinned (Pinned at ${pin?.y}, it at ${at?.y}, first ${shown[0]?.what})`);
    // Swipes, the pinned row's list as it is now.
    await sidebar(false); await sidebar(true);
    // A swipe across a row, at once: no drag.
    const r1 = await until("sidebar rows", () => rows()[2]);
    const [x, y] = L.mid(r1);
    // (Both ways: on 42cde3bc9 each of these dragged the row.)
    for (const dx of [120, -120]) {
      const swiped = await L.swipeWatching(x, y, x + L.px(dx), y, 300, dragging);
      await L.sleep(600);
      L.expect(!swiped, `a swipe across a row (${dx > 0 ? "right" : "left"}) dragged it (${swiped})`);
      await sidebar(true);
    }
    // A swipe along the list: it scrolls, nothing drags.
    const before = rows()[3];
    // (Up to three swipes: the list scrolls by one at least; none drags.)
    let after = before;
    for (let i = 0; i < 3 && after.y > before.y - 20; i++) {
      const sy = Math.round((before.y + sidebarBottom()) / 2);
      const scrolled = await L.swipeWatching(x, L.px(sy), x, L.px(sy - 200), 600, dragging);
      L.expect(!scrolled, `a swipe along the list dragged a row (${scrolled})`);
      await L.sleep(800);
      after = sidebarRows().find((n) => n.what === before.what) ?? { y: -1e9 };
    }
    L.expect(after.y < before.y - 20, `the list didn't scroll (${before.y} → ${after.y})`);
  },

  // The system's gestures take a finger that lands on their edge (home and
  // recents from the bottom, back from the left and right), sometimes after
  // it rested or moved, and cancel it for the app (ACTION_CANCEL): the touch
  // did nothing, no word selected, no drag, no tap, no keyboard.
  async systemGestures() {
    await openChat("E2E long reply");
    await hideKeyboard();
    const size = L.sh("wm size").match(/(\d+)x(\d+)\s*$/m);
    const [W, H] = [+size[1], +size[2]];
    const para = await until("a paragraph of the reply on screen", () => paragraphs().find((n) => n.y > 150 && n.y + 40 < 700));
    const py = L.px(para.y + 8);
    const cancel = (x, y) => L.sh(`input touchscreen motionevent CANCEL ${x} ${y}`);
    const moves = (x, y, dx, dy, n) => { for (let i = 1; i <= n; i++) L.move(Math.round(x + dx * i), Math.round(y + dy * i)); };
    // How often the system took a gesture from the app, as InputDispatcher logs it.
    const taken = () => (L.sh("logcat -d -s InputDispatcher:I", { ok: true }).match(/stealing input gesture/g) || []).length;
    const front = () => /mCurrentFocus=Window\{[^}]*sh\.zend\.zgx/.test(L.sh("dumpsys window | grep mCurrentFocus", { ok: true }));
    const activity = L.sh(`cmd package resolve-activity --brief ${L.PKG}`).trim().split("\n").pop();
    const traces = [];
    async function after(name, { system = false } = {}) {
      // (Back from another app the composer's keyboard comes up again, the
      // gesture or not: `input keyevent 3` and back does it too. Not a
      // gesture's trace, so not counted once the app went to the back.)
      let left_app = false;
      if (system) {
        await L.sleep(1200);
        if (!front()) { left_app = true; L.sh(`am start -n ${activity}`, { ok: true }); await L.sleep(2500); }
      }
      await L.sleep(800);
      const left = [];
      const p = popups();
      if (p.length) left.push(`selection (${p.length} popups)`);
      if (dragging()) left.push("a drag");
      if (openTitle() !== "E2E long reply") left.push(`opened ${openTitle()}`);
      if (sidebarShown()) left.push("the sidebar opened");
      if (!left_app && L.imeShown()) left.push("the keyboard up");
      if (left.length) traces.push(`${name}: ${left.join(", ")}`);
      // The next one starts clean: a tap on the reply clears a selection.
      if (popups().length) { L.tap(W / 2, L.px(300)); await L.sleep(600); }
      if (popups().length) { L.sh("input keyevent 4"); await L.sleep(600); }
      if (sidebarShown()) await sidebar(false);
      if (L.imeShown()) await hideKeyboard();
    }
    L.sh("logcat -c", { ok: true });
    // Taken after a rest on text, and after a move on it (motionevent CANCEL,
    // as the system sends it).
    { const [x, y] = [L.px(para.x + 40), py]; L.down(x, y); await L.sleep(900); cancel(x, y); await after("held on text, then cancelled"); }
    { const [x, y] = [L.px(para.x + 40), py]; L.down(x, y); moves(x, y, L.px(6), 0, 5); cancel(x + L.px(30), y); await after("moved on text, then cancelled"); }
    // The bottom edge: held, then cancelled; a real home swipe; held, then one.
    { const [x, y] = [W / 2, H - 4]; L.down(x, y); await L.sleep(900); cancel(x, y); await after("held on the bottom edge, then cancelled"); }
    const swipes = [];
    const real = async (name, fn) => {
      const was = taken();
      await fn();
      await L.sleep(800);
      if (taken() === was) traces.push(`${name}: the system didn't take it (no "stealing input gesture")`);
      await after(name, { system: true });
      swipes.push(name);
    };
    await real("home swipe from the bottom edge", async () => { L.swipe(W / 2, H - 2, W / 2, Math.round(H * 0.55), 400); });
    await real("held on the bottom edge, then a home swipe", async () => {
      const [x, y] = [W / 2, H - 4]; L.down(x, y); await L.sleep(900); moves(x, y, 0, -L.px(25), 10); L.up(x, y - L.px(250));
    });
    // Back from the left and right edges, the back zones widened so the
    // reply's text lies in them (as a person's wider back setting or a
    // phone's wider edge does): held on a word there, then swiped.
    try {
      L.sh("settings put secure back_gesture_inset_scale_left 2.0", { ok: true });
      L.sh("settings put secure back_gesture_inset_scale_right 2.0", { ok: true });
      await L.sleep(1500);
      const inLeft = L.px(para.x) + 15;
      const inRight = Math.min(L.px(para.x + para.w) - 20, W - 20);
      for (const [side, x, dx] of [["left", inLeft, 1], ["right", inRight, -1]]) {
        const y = py;
        { L.down(x, y); await L.sleep(900); cancel(x, y); await after(`held on text at the ${side} edge, then cancelled`); }
        await real(`back swipe from the ${side} edge`, async () => { L.swipe(x, y, x + dx * L.px(200), y, 300); });
        await real(`held on text at the ${side} edge, then a back swipe`, async () => {
          L.down(x, y); await L.sleep(900); moves(x, y, dx * L.px(20), 0, 10); L.up(x + dx * L.px(200), y);
        });
      }
    } finally {
      L.sh("settings delete secure back_gesture_inset_scale_left", { ok: true });
      L.sh("settings delete secure back_gesture_inset_scale_right", { ok: true });
    }
    L.expect(traces.length === 0, `a system gesture left a trace: ${traces.join("; ")}`);
  },
};

async function openFile(name) {
  // The project's files: the panel's File viewer, then the file in its tree.
  await openChat("E2E long reply");
  L.tap(...L.mid(await until("the Panel button", () => first('role:button "Panel"'))));
  await L.sleep(800); // (the panel slides in)
  L.tap(...L.mid(await until("the File viewer", () => first('role:button "File viewer"'))));
  const file = await until(`the file ${name}`, () => first(`role:treeitem "${name}"`));
  L.tap(...L.mid(file));
  await L.sleep(1500);
}
/** The reply's paragraphs as the tree lists them: [{ x, y, w, h }] in points. */
function paragraphs() {
  const text = L.inspect("tree", { budget: 600 }).text || "";
  return [...text.matchAll(/@\d+ text "Paragraph \d+:[^\n]*" (-?\d+),(-?\d+) (\d+)x(\d+)/g)].map((m) => ({ x: +m[1], y: +m[2], w: +m[3], h: +m[4] }));
}
/** The selection handles' frames on screen (pixels), from the window manager. */
function handleFrames() {
  const dump = L.sh("dumpsys window windows");
  const frames = [];
  for (const block of dump.split(/\n  Window #/)) {
    if (!/PopupWindow/.test(block) || !/sh\.zend\.zgx|package=sh\.zend\.zgx|mOwnerUid/.test(block)) continue;
    const m = block.match(/mFrame=\[(-?\d+),(-?\d+)\]\[(-?\d+),(-?\d+)\]/) || block.match(/frame=\[(-?\d+),(-?\d+)\]\[(-?\d+),(-?\d+)\]/);
    if (m) frames.push({ x: +m[1], y: +m[2], w: m[3] - m[1], h: m[4] - m[2] });
  }
  return frames.filter((f) => f.w > 0 && f.w < 400);
}

/** Each case from the app on top, its keyboard down: one case's slip (a link opened) never fails the next. */
// Each case starts from a fresh launch, so one case's leftovers (an open
// panel, a browser a stray tap opened) can't fail the next.
async function foreground() {
  L.sh(`am force-stop ${L.PKG}`, { ok: true });
  L.sh("am force-stop com.android.chrome", { ok: true });
  await S.start();
  await until("the app's devtools", () => { try { return L.inspect("windows").ok; } catch { return false; } }, 30000);
  // The sidebar's "Star on GitHub" chip, put away: a tap on it opens a browser.
  L.inspect("set", { name: "shell_star_banner_dismissed", value: true });
  await until("the composer", () => composer(), 20000);
  await hideKeyboard().catch(() => {});
}

// ---- the run

console.log(`e2e: ${L.SERIAL}, apk ${apk ?? "(installed)"}, out ${out}`);
L.sh(`settings put global hide_error_dialogs 1`, { ok: true });
L.sh(`ime enable ${L.IME}`); L.sh(`ime set ${L.IME}`);
const e = await S.engine(out);
for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"]) process.on(sig, () => { e.stop(); process.exit(130); });
console.log(`engine ${e.ip}:${e.port}`);
const seeded = await S.seed(e);
e.rowIds = seeded.rows;
await S.app(e, apk);
L.sh(`pm grant ${L.PKG} android.permission.POST_NOTIFICATIONS`, { ok: true });
await L.sleep(2000);
for (const [name, fn] of Object.entries(cases)) {
  if (only && !only.includes(name)) continue;
  await L.test(name, async () => { await foreground(); await fn(e); });
}
const ok = L.summary();
if (keep) { console.log("kept: engine and app stay up (Ctrl-C to end)"); await new Promise(() => {}); }
e.stop();
process.exit(ok ? 0 : 1);
