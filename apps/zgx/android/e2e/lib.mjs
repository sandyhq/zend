// The e2e suite's hands and eyes on the emulator: adb for real input (touch
// through `input motionevent`, text through the test keyboard's
// InputConnection), zgx's devtools socket for what the app shows (its tree,
// node boxes, states), logcat for the keyboard's and the app's reports.
import { spawnSync, spawn } from "node:child_process";
import fs from "node:fs";

export const PKG = "sh.zend.zgx";
const SDK = process.env.ANDROID_HOME || process.env.HOME + "/Android/Sdk";
export const ADB = SDK + "/platform-tools/adb";
export const SERIAL = process.env.ANDROID_SERIAL || "emulator-5554";

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** `adb -s SERIAL <args>`: its stdout (throws on failure unless ok). */
export function adb(args, { input, ok = false, timeout = 60000, encoding = "utf8" } = {}) {
  const r = spawnSync(ADB, ["-s", SERIAL, ...args], { input, encoding: encoding === "buffer" ? null : encoding, timeout, maxBuffer: 64 << 20 });
  if (r.status !== 0 && !ok) throw new Error(`adb ${args.join(" ")}: ${r.status} ${r.stderr || r.stdout}`);
  return r.stdout ?? "";
}
export const sh = (cmd, o) => adb(["shell", cmd], o);

// ---- the app's devtools socket (apps/zgx/src/android.rs: files/zgx-inspect)

let uid = null;
export function appPid() {
  return sh(`pidof ${PKG}`, { ok: true }).trim().split(/\s+/)[0] || null;
}
function sockPath(pid) {
  if (uid == null) uid = sh(`run-as ${PKG} id -u`).trim();
  return `cache/zgx-inspect-${uid}/${pid}.sock`;
}

/** One devtools request: `{ ok, text, data }` (see zgx's inspect.md). */
export function inspect(cmd, fields = {}) {
  const pid = appPid();
  if (!pid) throw new Error("the app isn't running");
  const out = adb(["shell", "run-as", PKG, "nc", "-q", "5", "-U", sockPath(pid)], {
    input: JSON.stringify({ cmd, ...fields }) + "\n",
    ok: true,
  });
  const line = out.split("\n").find((l) => l.startsWith("{"));
  if (!line) throw new Error(`inspect ${cmd}: no answer (${out.trim().slice(0, 200)})`);
  return JSON.parse(line);
}

/** Nodes a target matches: [{ id, x, y, w, h, text }] in points. */
export function find(target) {
  const r = inspect("find", { target });
  if (!r.ok) return [];
  return (r.text || "").split("\n").filter(Boolean).map((l) => {
    const m = l.match(/^@(\d+) (.*?) (-?\d+),(-?\d+) (\d+)x(\d+)/);
    return m ? { id: +m[1], what: m[2], x: +m[3], y: +m[4], w: +m[5], h: +m[6], line: l } : { line: l };
  });
}
export function node(target) {
  const r = inspect("node", { target });
  return r.ok ? r.data : null;
}
/** A shared state's value (devtools answers `{ [name]: value }`). */
export function state(name) {
  const r = inspect("state", { name, full: true });
  return r.ok ? r.data?.[name] ?? null : null;
}

// ---- points and pixels

let scale = null;
/** Window points to screen pixels (the app draws edge to edge). */
export function px(pt) {
  if (scale == null) {
    const w = inspect("windows").data[0].width;
    const size = sh("wm size").match(/(\d+)x(\d+)\s*$/m);
    scale = +size[1] / w;
  }
  return Math.round(pt * scale);
}
export const mid = (n) => [px(n.x + n.w / 2), px(n.y + n.h / 2)];

// ---- touch, as a finger does: real MotionEvents through the input system

export function down(x, y) { sh(`input touchscreen motionevent DOWN ${x} ${y}`); }
export function move(x, y) { sh(`input touchscreen motionevent MOVE ${x} ${y}`); }
export function up(x, y) { sh(`input touchscreen motionevent UP ${x} ${y}`); }
export function tap(x, y) { sh(`input touchscreen tap ${x} ${y}`); }
/** A swipe of `ms` (input's own, evenly spaced moves). */
export function swipe(x1, y1, x2, y2, ms = 300) { sh(`input touchscreen swipe ${x1} ${y1} ${x2} ${y2} ${ms}`); }
/**
 * A swipe of `ms` as one gesture (`input swipe`: each `input` command
 * costs a few hundred ms, so moves sent one by one make a finger that rests
 * long enough to be held), with `probe` asked meanwhile: its first truthy
 * answer, or false.
 */
export async function swipeWatching(x1, y1, x2, y2, ms, probe) {
  const child = spawn(ADB, ["-s", SERIAL, "shell", `input touchscreen swipe ${x1} ${y1} ${x2} ${y2} ${ms}`]);
  const done = new Promise((r) => child.on("exit", r));
  let ended = false;
  done.then(() => (ended = true));
  let seen = false;
  while (!ended) {
    try { seen = seen || probe(); } catch {}
    await sleep(30);
  }
  return seen;
}
/** Down, still for `holdMs`, then moved to (x2, y2) in `steps` and up. */
export async function holdThenDrag(x, y, x2, y2, holdMs, steps = 12) {
  down(x, y);
  await sleep(holdMs);
  for (let i = 1; i <= steps; i++) move(Math.round(x + ((x2 - x) * i) / steps), Math.round(y + ((y2 - y) * i) / steps));
  await sleep(150);
  up(x2, y2);
}

// ---- the test keyboard (ime/: sh.zend.e2e.ime)

export const IME = "sh.zend.e2e.ime/.TestIme";
let seq = 0;
const b64 = (s) => Buffer.from(s, "utf8").toString("base64");
export const unb64 = (s) => (s == null || s === "null" ? null : Buffer.from(s, "base64").toString("utf8"));

/** Ask the keyboard: its `result` line as fields (text fields decoded). */
export async function ime(op, { text, n } = {}) {
  const id = `${process.pid}-${++seq}`;
  let cmd = `am broadcast -a sh.zend.e2e.ime.CMD -p sh.zend.e2e.ime --es op ${op} --es seq ${id}`;
  if (text != null) cmd += ` --es b64 ${b64(text)}`;
  if (n != null) cmd += ` --ei n ${n}`;
  sh(cmd);
  for (let i = 0; i < 40; i++) {
    const line = sh(`logcat -d -s ZendE2eIme:I`).split("\n").find((l) => l.includes(`result seq=${id} `));
    if (line) {
      const f = {};
      for (const m of line.matchAll(/(\w+)=(\S+)/g)) f[m[1]] = m[2];
      for (const k of ["before", "after", "clip", "extracted"]) if (k in f) f[k] = unb64(f[k]);
      f.ok = f.ok === "true";
      return f;
    }
    await sleep(100);
  }
  throw new Error(`the keyboard didn't answer ${op}`);
}

/** Whether the soft keyboard shows now (InputMethodManagerService's view). */
export function imeShown() {
  return /mInputShown=true/.test(sh("dumpsys input_method"));
}

/** Samples imeShown every `every` ms while `fn` runs and `tail` ms after. */
export async function watchIme(fn, { every = 150, tail = 1500 } = {}) {
  const seen = [];
  let on = true;
  const loop = (async () => { while (on) { seen.push(imeShown()); await sleep(every); } })();
  await fn();
  await sleep(tail);
  on = false;
  await loop;
  return seen;
}
/** How often a run of samples goes from hidden to shown. */
export const reshows = (seen) => seen.reduce((n, v, i) => n + (i > 0 && v && !seen[i - 1] ? 1 : 0), 0);

// ---- reporting

const results = [];
export async function test(name, fn) {
  const t0 = Date.now();
  try {
    await fn();
    results.push({ name, ok: true });
    console.log(`  ok    ${name}  (${((Date.now() - t0) / 1000).toFixed(1)} s)`);
  } catch (e) {
    results.push({ name, ok: false, why: e.message });
    console.log(`  FAIL  ${name}\n        ${String(e.message).split("\n").join("\n        ")}`);
    try { fs.writeFileSync(`${process.env.E2E_OUT || "/tmp"}/fail-${results.length}.png`, adb(["exec-out", "screencap", "-p"], { encoding: "buffer" })); } catch {}
  }
}
export function expect(cond, msg) { if (!cond) throw new Error(msg); }
export function eq(a, b, what) {
  if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`${what}: expected ${JSON.stringify(b)}, got ${JSON.stringify(a)}`);
}
export function summary() {
  const failed = results.filter((r) => !r.ok);
  console.log(`\n${results.length - failed.length}/${results.length} passed${failed.length ? ": failed " + failed.map((r) => r.name).join("; ") : ""}`);
  return failed.length === 0;
}
