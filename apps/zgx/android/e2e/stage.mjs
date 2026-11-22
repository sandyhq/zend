// The suite's computer: a stand-in Zend engine of its own (fresh data, its
// phone link on), a stand-in model (mock-model.mjs), seeded chats, and the
// app on the emulator linked to that engine as a phone is by its code.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import * as L from "./lib.mjs";

const here = path.dirname(new URL(import.meta.url).pathname);

export async function engine(out) {
  const bin = process.env.ZEND_ENGINE || process.env.HOME + "/.cache/android-app/direct-run/zend-engine";
  const port = +(process.env.E2E_ENGINE_PORT || 27961);
  const mockPort = +(process.env.E2E_MOCK_PORT || 27962);
  const run = path.join(out, "computer");
  fs.rmSync(run, { recursive: true, force: true });
  const home = path.join(run, "home"), data = path.join(run, "engine"), project = path.join(run, "project");
  for (const d of [home + "/.pi/agent", data, project]) fs.mkdirSync(d, { recursive: true });
  fs.writeFileSync(home + "/.pi/agent/models.json", JSON.stringify({ providers: { mock: { baseUrl: `http://127.0.0.1:${mockPort}/v1`, api: "openai-completions", apiKey: "mock", models: [{ id: "long", name: "Mock long", contextWindow: 128000, maxTokens: 8000 }] } } }, null, 2));
  fs.writeFileSync(data + "/phone-link.json", JSON.stringify({ on: true, use: "ip" }));
  spawnSync("git", ["init", "-q", "-b", "main", project]);
  fs.writeFileSync(project + "/README.md", "e2e project\n");
  spawnSync("git", ["-C", project, "add", "-A"]);
  spawnSync("git", ["-C", project, "-c", "user.email=e2e@zend", "-c", "user.name=e2e", "-c", "commit.gpgsign=false", "commit", "-qm", "init"]);
  const mockLog = path.join(run, "model-requests.jsonl");
  fs.writeFileSync(mockLog, "");
  const nice = (cmd, args, env, log) => spawn("nice", ["-n", "19", "ionice", "-c3", cmd, ...args], { env: { ...process.env, ...env }, stdio: ["ignore", fs.openSync(log, "w"), fs.openSync(log, "a")], detached: false });
  const mock = nice("node", [here + "/mock-model.mjs", String(mockPort)], { MOCK_LOG: mockLog }, path.join(run, "mock.log"));
  const env = { HOME: home, ZEND_DATA_DIR: data, ZEND_IPC_PORT: String(port), PATH: process.env.PATH };
  const eng = nice(bin, ["headless"], env, path.join(run, "engine.log"));
  // Its phone link's address: the tailnet's (the link listens nowhere else).
  let ip = null;
  for (let i = 0; i < 120 && !ip; i++) {
    await L.sleep(500);
    const log = fs.readFileSync(path.join(run, "engine.log"), "utf8").replace(/\x1b\[[0-9;]*m/g, "");
    ip = log.match(/phone link: listening addr=(\d+\.\d+\.\d+\.\d+):(\d+)/)?.[1];
    if (/no_tailnet|no tailnet/i.test(log)) throw new Error("the stand-in engine found no tailnet: its phone link needs one (tailscale up)");
  }
  if (!ip) throw new Error("the stand-in engine's phone link didn't start (see " + run + "/engine.log)");
  const key = fs.readFileSync(data + "/phone-link-key", "utf8").trim();
  const mcp = mcpClient(bin, env);
  process.on("exit", () => { for (const p of [eng, mock, mcp.proc]) try { p.kill("SIGTERM"); } catch {} });
  await mcp.ready;
  return {
    run, port, ip, key, project, mockLog, mcp,
    requests: () => fs.readFileSync(mockLog, "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l)),
    stop() { mcp.close(); for (const p of [eng, mock]) try { p.kill("SIGTERM"); } catch {} },
  };
}

/** The engine's MCP server (zend-engine mcp): call(tool, args) → parsed JSON or text. */
function mcpClient(bin, env) {
  const p = spawn(bin, ["mcp"], { env, stdio: ["pipe", "pipe", "ignore"] });
  let buf = "", n = 0;
  const waits = new Map();
  p.stdout.on("data", (d) => {
    buf += d;
    let i;
    while ((i = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, i);
      buf = buf.slice(i + 1);
      try { const m = JSON.parse(line); if (waits.has(m.id)) { waits.get(m.id)(m); waits.delete(m.id); } } catch {}
    }
  });
  const rpc = (method, params) => new Promise((r) => { const id = ++n; waits.set(id, r); p.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n"); });
  const ready = (async () => {
    await rpc("initialize", { protocolVersion: "2025-03-26", capabilities: {}, clientInfo: { name: "e2e", version: "1" } });
    p.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
  })();
  return {
    proc: p,
    ready,
    async call(name, args) {
      const m = await rpc("tools/call", { name, arguments: args });
      if (m.error || m.result?.isError) throw new Error(`engine ${name}: ${JSON.stringify(m.error || m.result).slice(0, 400)}`);
      const text = (m.result?.content || []).map((c) => c.text).join("");
      try { return JSON.parse(text); } catch { return text; }
    },
    close() { try { p.kill(); } catch {} },
  };
}

/** Chats for the cases: one with a long reply (the stand-in model's), and
 *  enough empty ones that the sidebar's list scrolls. */
export async function seed(e) {
  // (Right after its start the engine's model catalog may still be looking:
  // "retry" is its word for that.)
  for (let i = 0; ; i++) {
    try { await e.mcp.call("list_models", { harness: "pi" }); break; } catch (err) { if (i > 60) throw err; await L.sleep(1000); }
  }
  // (Rows enough that the sidebar's list scrolls, then the long reply: the
  // newest, at the list's top, where a case finds it without scrolling.)
  const rows = [];
  for (let i = 1; i <= 30; i++) {
    const c = await e.mcp.call("create_chat", { kind: "chat", title: `E2E row ${String(i).padStart(2, "0")}`, harness: "pi", model: "mock/long", cwd: e.project });
    rows.push(c.chatId || c.id || c.chat?.id);
  }
  const long = await e.mcp.call("create_chat", { kind: "chat", title: "E2E long reply", harness: "pi", model: "mock/long", cwd: e.project, prompt: "Write a long answer about foxes.", wait: true, timeout_secs: 120 });
  const id = long.chatId || long.id || long.chat?.id;
  return { long: id, rows };
}

/** The app fresh (its data cleared), devtools on, linked to `e`, started. */
export async function app(e, apk) {
  if (apk) L.adb(["install", "-r", "-g", apk], { timeout: 300000 });
  L.sh(`pm clear ${L.PKG}`);
  // (The app starts only in a private files folder: made as Android makes
  // it, 0771, its own folders 0700.)
  L.sh(`run-as ${L.PKG} sh -c 'umask 077; mkdir -p files/.config/zend && chmod 771 files && touch files/zgx-inspect'`);
  L.adb(["shell", "run-as", L.PKG, "sh", "-c", "'umask 077; cat > files/.config/zend/desktop'"], { input: `zend-desktop 1\n${e.ip}\n${e.port}\n${e.key}\n` });
  return start();
}

/** Starts the app and waits until its devtools answer. */
export async function start() {
  L.sh(`am force-stop ${L.PKG}`);
  // (Its launcher activity, whatever its class: NativeActivity or the app's own.)
  const activity = L.sh(`cmd package resolve-activity --brief ${L.PKG}`).trim().split("\n").pop();
  L.sh(`am start -W -n ${activity}`);
  for (let i = 0; i < 120; i++) {
    await L.sleep(500);
    try { if (L.inspect("windows").ok) return; } catch {}
  }
  throw new Error("the app's devtools never answered (files/zgx-inspect, a debuggable build?)");
}
