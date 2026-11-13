/**
 * Runs the real Cloud install script (src/cloud/install-script.ts) under sh
 * in a temp dir, with `curl`/`sudo`/`systemctl`/`flock`/`uname` stubbed on
 * PATH, to prove what a workflow step retry relies on: re-running converges
 * without re-downloading or restarting, the env file is 0600, and a changed
 * enrollment value (re-provision) rewrites it and restarts the service.
 */
import { execFileSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readlinkSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir, userInfo } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  CLOUD_BOOT_UNIT,
  CLOUD_INSTALL_SCRIPT,
  START_ENGINE_COMMAND,
  STOP_ENGINE_COMMAND,
  installCommand,
  type SessionEnv
} from "../../src/cloud/install-script";

/** One session's provisioning (the workflow's create env). */
const session = (enroll: string, edge = "https://edge.test", overrides: Partial<SessionEnv> = {}): SessionEnv => ({
  ZEND_RUNNER_ENROLL: enroll,
  ZEND_EDGE_URL: edge,
  ZEND_DEVICE_NAME: "Cloud session",
  ZEND_DEVICE_PLATFORM: "cloud",
  ZEND_CLOUD_ACCOUNT: "cloud-account-1",
  ZEND_CLOUD_CHAT: "chat-1",
  ZEND_CLOUD_REPO: "https://github.com/acme/app.git",
  ZEND_CLOUD_PATH: "/home/user/app",
  ZEND_CLOUD_BRANCH: "main",
  ...overrides
});

let root: string;

const stub = (bin: string, name: string, body: string) => {
  const path = join(bin, name);
  writeFileSync(path, `#!/bin/sh\n${body}\n`);
  chmodSync(path, 0o755);
};

const setup = () => {
  root = mkdtempSync(join(tmpdir(), "zend-cloud-install-"));
  const bin = join(root, "bin");
  const home = join(root, "home");
  const units = join(root, "units");
  const release = join(root, "release");
  for (const dir of [bin, home, units, join(release, "zend-1.2.3-linux-x86_64")]) mkdirSync(dir, { recursive: true });
  const fakeZend = join(release, "zend-1.2.3-linux-x86_64", "zend");
  writeFileSync(fakeZend, "#!/bin/sh\necho zend 1.2.3\n");
  chmodSync(fakeZend, 0o755);
  // The release is the desktop's too: it carries the background engine.
  writeFileSync(join(release, "zend-1.2.3-linux-x86_64", "zend-engine"), "#!/bin/sh\nexit 1\n");
  execFileSync("tar", ["-czf", join(release, "zend.tar.gz"), "-C", release, "zend-1.2.3-linux-x86_64"]);
  const log = join(root, "calls.log");
  writeFileSync(log, "");
  // curl: latest.txt prints the version; the tarball is copied to -o.
  stub(bin, "curl", `echo "curl $*" >> "${log}"
out=""; url=""
while [ $# -gt 0 ]; do case "$1" in -o) out="$2"; shift 2;; -*) shift;; *) url="$1"; shift;; esac; done
case "$url" in
  */releases/latest.txt) echo "1.2.3" ;;
  */releases/zend-1.2.3-linux-x86_64.tar.gz) cp "${join(release, "zend.tar.gz")}" "$out" ;;
  *) exit 22 ;;
esac`);
  stub(bin, "sudo", `"$@"`);
  stub(bin, "systemctl", `echo "systemctl $*" >> "${log}"`);
  stub(bin, "sync", `echo "sync" >> "${log}"`);
  stub(bin, "flock", "exit 0");
  stub(bin, "uname", `[ "$1" = "-m" ] && echo x86_64 || /usr/bin/uname "$@"`);
  const script = join(root, "install.sh");
  writeFileSync(script, CLOUD_INSTALL_SCRIPT);
  const run = (enroll: string) =>
    execFileSync("sh", [script], {
      env: {
        PATH: `${bin}:/usr/bin:/bin`,
        ...session(enroll),
        ZEND_CLOUD_HOME: home,
        ZEND_CLOUD_UNIT_DIR: units
      },
      encoding: "utf8"
    });
  const calls = () => readFileSync(log, "utf8").trim().split("\n").filter(Boolean);
  const clearCalls = () => writeFileSync(log, "");
  return { home, units, run, calls, clearCalls };
};

beforeEach(() => {
  root = "";
});
afterEach(() => {
  if (root) rmSync(root, { recursive: true, force: true });
});

// These run the real script under a shell several times: slow on a busy machine.
describe("cloud install script", { timeout: 60_000 }, () => {
  it("is ASCII (it travels base64-encoded via btoa) and carries its idempotency guards", () => {
    expect(/^[\x00-\x7f]*$/.test(CLOUD_INSTALL_SCRIPT)).toBe(true);
    for (const marker of ["#!/bin/sh", "set -eu", "umask 077", "flock 9", 'if [ -x "$dest/zend" ]', "ln -sfn", "chmod 600", "cmp -s", "systemctl enable", "EnvironmentFile="]) {
      expect(CLOUD_INSTALL_SCRIPT).toContain(marker);
    }
  });

  it("installs once, converges on re-run, and restarts only on change", () => {
    const { home, units, run, calls, clearCalls } = setup();
    run("org.user.cloud-1.codeA");

    const data = join(home, ".zend");
    expect(readlinkSync(join(data, "app", "current"))).toBe(join(data, "app", "1.2.3"));
    expect(existsSync(join(data, "app", "1.2.3", "zend"))).toBe(true);
    // Never run on Cloud, and copied back in on every wake if kept.
    expect(existsSync(join(data, "app", "1.2.3", "zend-engine"))).toBe(false);
    const envFile = join(data, "env");
    expect(statSync(envFile).mode & 0o777).toBe(0o600);
    expect(readFileSync(envFile, "utf8")).toBe(
      [
        "ZEND_RUNNER_ENROLL=org.user.cloud-1.codeA",
        "ZEND_EDGE_URL=https://edge.test",
        'ZEND_DEVICE_NAME="Cloud session"',
        "ZEND_DEVICE_PLATFORM=cloud",
        "ZEND_CLOUD_ACCOUNT=cloud-account-1",
        "ZEND_CLOUD_CHAT=chat-1",
        "ZEND_CLOUD_REPO=https://github.com/acme/app.git",
        "ZEND_CLOUD_PATH=/home/user/app",
        "ZEND_CLOUD_BRANCH=main",
        "ZEND_AUTO_UPDATE=1",
        `HOME=${home}`,
        ""
      ].join("\n")
    );
    const unit = readFileSync(join(units, "zend-cloud.service"), "utf8");
    expect(statSync(join(units, "zend-cloud.service")).mode & 0o777).toBe(0o644);
    expect(unit).toContain(`User=${userInfo().username}`);
    expect(unit).toContain(`WorkingDirectory=${home}`);
    expect(unit).toContain(`EnvironmentFile=${envFile}`);
    expect(unit).toContain(`ExecStart=${join(data, "app", "current", "zend")} headless`);
    expect(unit).toContain("Restart=always");
    expect(calls().filter((c) => c.startsWith("curl"))).toHaveLength(2);
    // A stalled download is dropped and retried, never waited on forever.
    expect(calls().find((c) => c.includes(".tar.gz"))).toMatch(/--retry 3 .*--speed-limit \d+ --speed-time \d+/);
    expect(calls()).toContain("systemctl restart zend-cloud.service");

    // A retried step: nothing to download, nothing changed, no restart.
    clearCalls();
    run("org.user.cloud-1.codeA");
    expect(calls().filter((c) => c.startsWith("curl"))).toHaveLength(1); // latest.txt only
    expect(calls()).toContain("systemctl start zend-cloud.service");
    expect(calls()).not.toContain("systemctl restart zend-cloud.service");
    expect(calls()).toContain("systemctl enable zend-cloud.service zend-cloud-boot.service");
    expect(calls()).toContain("systemctl start zend-cloud-boot.service");

    // A re-provision with a fresh code rewrites the env file and restarts.
    clearCalls();
    run("org.user.cloud-1.codeB");
    expect(readFileSync(envFile, "utf8")).toContain("ZEND_RUNNER_ENROLL=org.user.cloud-1.codeB\n");
    expect(statSync(envFile).mode & 0o777).toBe(0o600);
    expect(calls()).toContain("systemctl restart zend-cloud.service");
  });

  it("doesn't wait for the network to start the engine, and starts it on boot without being asked", () => {
    const { units, run } = setup();
    run("org.user.cloud-1.codeA");
    const unit = readFileSync(join(units, "zend-cloud.service"), "utf8");
    expect(unit).not.toContain("network-online");
    expect(unit).toContain("RestartSec=1\n");
    // A fast restart must not trip systemd's default start limit for good.
    expect(unit).toContain("StartLimitIntervalSec=0\n");
    const boot = readFileSync(join(units, "zend-cloud-boot.service"), "utf8");
    expect(boot).toBe(CLOUD_BOOT_UNIT);
    expect(boot).toContain("RemainAfterExit=yes\n");
    expect(boot).toContain("ExecStart=/bin/systemctl start --no-block zend-cloud.service\n");
    expect(boot).toContain("WantedBy=multi-user.target\n");
  });

  describe("the sleep's stop command", () => {
    /** A machine from before these units: its unit as the old installer wrote it. */
    const OLD_UNIT = (fresh: string) =>
      fresh
        .replace("StartLimitIntervalSec=0\n", "After=network-online.target\nWants=network-online.target\n")
        .replace("RestartSec=1\n", "RestartSec=5\n");
    const stop = (env: Record<string, string>) =>
      execFileSync("sh", ["-c", STOP_ENGINE_COMMAND], {
        env: { PATH: `${join(root, "bin")}:/usr/bin:/bin`, ...env },
        encoding: "utf8"
      });

    it("brings an older machine's units up to date before the snapshot, then stops the engine", () => {
      const { units, run, calls, clearCalls } = setup();
      run("org.user.cloud-1.codeA");
      const path = join(units, "zend-cloud.service");
      const fresh = readFileSync(path, "utf8");
      writeFileSync(path, OLD_UNIT(fresh));
      rmSync(join(units, "zend-cloud-boot.service"));
      clearCalls();

      stop({ ZEND_CLOUD_UNIT_DIR: units });

      // What a new machine gets, byte for byte.
      expect(readFileSync(path, "utf8")).toBe(fresh);
      expect(readFileSync(join(units, "zend-cloud-boot.service"), "utf8")).toBe(CLOUD_BOOT_UNIT);
      expect(calls()).toEqual([
        "systemctl daemon-reload",
        "systemctl enable zend-cloud-boot.service",
        "systemctl start zend-cloud-boot.service",
        "systemctl stop zend-cloud.service",
        // Written out before the provider's snapshot.
        "sync"
      ]);
      // Run again: same files, and the engine still stops last, then the flush.
      stop({ ZEND_CLOUD_UNIT_DIR: units });
      expect(readFileSync(path, "utf8")).toBe(fresh);
      expect(calls().slice(-2)).toEqual(["systemctl stop zend-cloud.service", "sync"]);
    });

    it("still stops the engine when nothing can be upgraded, and answers with the stop's status", () => {
      const { units, calls, clearCalls } = setup();
      clearCalls();
      stop({ ZEND_CLOUD_UNIT_DIR: units }); // no unit file at all
      expect(calls()).toEqual(["systemctl stop zend-cloud.service", "sync"]);

      // A unit whose rewrite would lose its command is left as it is.
      const path = join(units, "zend-cloud.service");
      writeFileSync(path, "[Unit]\nDescription=Zend Cloud engine\n");
      clearCalls();
      stop({ ZEND_CLOUD_UNIT_DIR: units });
      expect(readFileSync(path, "utf8")).toBe("[Unit]\nDescription=Zend Cloud engine\n");
      expect(calls().slice(-2)).toEqual(["systemctl stop zend-cloud.service", "sync"]);

      writeFileSync(join(root, "bin", "systemctl"), `#!/bin/sh\ncase "$1" in stop) exit 3;; *) exit 0;; esac\n`);
      clearCalls();
      expect(() => stop({ ZEND_CLOUD_UNIT_DIR: units })).toThrow();
      // A failed stop still flushes, and still answers with the stop's status.
      expect(calls().at(-1)).toBe("sync");
    });
  });

  it("starts the engine without waiting on it, and reloads once for a unit systemd hasn't read", () => {
    const { calls, clearCalls } = setup();
    const start = () =>
      execFileSync("sh", ["-c", START_ENGINE_COMMAND], { env: { PATH: `${join(root, "bin")}:/usr/bin:/bin` }, encoding: "utf8" });
    clearCalls();
    start();
    expect(calls()).toEqual(["systemctl start --no-block zend-cloud.service"]);

    // The first start fails (Unit not found) until a daemon-reload has run.
    writeFileSync(
      join(root, "bin", "systemctl"),
      `#!/bin/sh\necho "systemctl $*" >> "${join(root, "calls.log")}"\ncase "$1" in\n  daemon-reload) touch "${join(root, "reloaded")}";;\n  start) [ -f "${join(root, "reloaded")}" ] || exit 5;;\nesac\n`
    );
    clearCalls();
    start();
    expect(calls()).toEqual([
      "systemctl start --no-block zend-cloud.service",
      "systemctl daemon-reload",
      "systemctl start --no-block zend-cloud.service"
    ]);
  });

  it("builds a quote-safe command and refuses values that could break out", () => {
    const command = installCommand(session("org_1.user_1.cloud-abc.Zm9v-_x", "https://edge.zend.invalid"));
    expect(command).toContain("ZEND_RUNNER_ENROLL='org_1.user_1.cloud-abc.Zm9v-_x'");
    expect(command).toContain("ZEND_EDGE_URL='https://edge.zend.invalid'");
    expect(command).toContain("ZEND_CLOUD_REPO='https://github.com/acme/app.git'");
    expect(() => installCommand(session("org.user.dev.co'de", "https://edge.zend.invalid"))).toThrow();
    expect(() => installCommand(session("org.user.dev.code", "https://edge.zend.invalid/x'; rm -rf /"))).toThrow();
    expect(() => installCommand(session("org.user.dev.code", "https://edge.test", { ZEND_CLOUD_REPO: "https://github.com/a/b'; id" }))).toThrow();
    expect(() => installCommand(session("org.user.dev.code", "https://edge.test", { ZEND_CLOUD_PATH: "/home/user/../../etc" }))).toThrow();
    expect(() => installCommand(session("org.user.dev.code", "https://edge.test", { ZEND_CLOUD_BRANCH: "main; reboot" }))).toThrow();
  });

  it("runs end to end through the command wrapper", () => {
    const { home, units } = setup();
    const command = installCommand(session("org.user.cloud-1.codeC"));
    execFileSync("sh", ["-c", command], {
      env: {
        PATH: `${join(root, "bin")}:/usr/bin:/bin`,
        ZEND_CLOUD_HOME: home,
        ZEND_CLOUD_UNIT_DIR: units
      },
      encoding: "utf8"
    });
    expect(readFileSync(join(home, ".zend", "env"), "utf8")).toContain("ZEND_RUNNER_ENROLL=org.user.cloud-1.codeC\n");
  });
});
