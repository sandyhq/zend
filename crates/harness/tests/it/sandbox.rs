//! The bubblewrap sandbox, for real: what a confined process can see, write
//! and inherit, and where it can connect. Needs `bwrap` and `pasta` (skipped,
//! with a note, where one is missing).
#![cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
use std::process::Stdio;
use zend_harness::sandbox::{Host, Launch, Sandbox, logins};
use zend_proto::HarnessId;

fn bwrap() -> Option<PathBuf> {
    let path = PathBuf::from("/usr/bin/bwrap");
    if path.is_file() && Path::new("/usr/bin/pasta").is_file() {
        return Some(path);
    }
    eprintln!("bwrap or pasta not installed: skipped");
    None
}

fn host(fake: &FakeHome, bwrap: PathBuf) -> Host {
    Host {
        home: fake.home.clone(),
        bwrap: Some(bwrap),
        pasta: None,
        hidden: Vec::new(),
    }
}

/// A home with what must stay hidden, two projects and a Pi login.
struct FakeHome {
    _dir: tempfile::TempDir,
    home: PathBuf,
    project: PathBuf,
    sibling: PathBuf,
}

fn fake_home() -> FakeHome {
    let dir = tempfile::tempdir().unwrap();
    let home = std::fs::canonicalize(dir.path()).unwrap().join("home");
    let write = |path: &str, text: &str| {
        let path = home.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(".ssh/id_ed25519", "PRIVATE KEY");
    write(".zend/ipc-token", "ZEND TOKEN");
    write(".config/zend/settings.json", "{}");
    write(".pi/agent/auth.json", "{\"login\":\"pi\"}");
    write(".pi/agent/settings.json", "{\"private\":true}");
    write("notes.txt", "rest of home");
    write("projects/a/README", "project a");
    write("projects/b/secret", "project b");
    FakeHome {
        project: home.join("projects/a"),
        sibling: home.join("projects/b"),
        home,
        _dir: dir,
    }
}

/// Run `script` with sh in a sandbox of the fake home's project a, with
/// Pi's logins; return (exit code, stdout, stderr).
async fn run_in(fake: &FakeHome, bwrap: PathBuf, script: &str) -> (i32, String, String) {
    run_as(fake, bwrap, Path::new("/bin/sh"), script).await
}

/// [`run_in`] with `exe` as the agent the launch is made for.
async fn run_as(fake: &FakeHome, bwrap: PathBuf, exe: &Path, script: &str) -> (i32, String, String) {
    let sandbox = Sandbox::project(&fake.project, host(fake, bwrap)).unwrap();
    let pi_logins = logins(HarnessId::Pi, &fake.home, &|_| None);
    let launch = Launch::new(&sandbox, HarnessId::Pi, exe, &fake.project, &pi_logins).unwrap();
    let mut command = launch.command("/bin/sh");
    command
        .arg("-c")
        .arg(script)
        // What a driver would have set: never reaches the agent.
        .env("SSH_AUTH_SOCK", "/run/user/1000/keyring/ssh")
        .env("ZEND_IPC_TOKEN", "secret")
        .env("ZEND_ORIGIN", "chat:x")
        .env("PI_CODING_AGENT_DIR", fake.home.join(".pi/agent"))
        // As a driver sets it (compose_child_path): other tests in this
        // process change the PATH it would inherit.
        .env("PATH", "/usr/bin:/bin")
        .current_dir(&fake.project)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = launch.spawn(&mut command).unwrap();
    let output = child.wait_with_output().await.unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[tokio::test]
async fn a_sandboxed_agent_sees_only_its_project_and_its_login() {
    let Some(bwrap) = bwrap() else { return };
    let fake = fake_home();
    let h = fake.home.display();
    let b = fake.sibling.display();
    let script = format!(
        r#"
        for f in {h}/.ssh/id_ed25519 {h}/.zend/ipc-token {h}/.config/zend/settings.json \
                 {h}/notes.txt {h}/.pi/agent/settings.json {b}/secret /var/run/docker.sock /run/k3s; do
            if cat "$f" >/dev/null 2>&1; then echo "READABLE $f"; fi
        done
        ls {h}/projects
        cat {h}/.pi/agent/auth.json; echo
        (echo hi > {h}/.pi/agent/auth.json) 2>/dev/null && echo "LOGIN WRITABLE"
        echo written > ./out.txt && echo "wrote project"
        echo tmp > "$HOME/scratch" && echo "home is $HOME, writable"
        pwd
        "#
    );
    let (code, out, err) = run_in(&fake, bwrap, &script).await;
    println!("exit {code}\n--- stdout\n{out}--- stderr\n{err}");
    assert_eq!(code, 0, "{err}");
    assert!(!out.contains("READABLE"), "{out}");
    assert!(!out.contains("LOGIN WRITABLE"), "{out}");
    // Only the granted project in projects/: no sibling.
    assert!(out.lines().any(|line| line == "a"), "{out}");
    assert!(!out.lines().any(|line| line == "b"), "{out}");
    assert!(out.contains("{\"login\":\"pi\"}"), "{out}");
    assert!(out.contains("wrote project"), "{out}");
    assert!(out.contains(&format!("home is {h}, writable")), "{out}");
    assert!(out.contains(&fake.project.display().to_string()), "{out}");
    // Written through to the host's project; the home write stayed inside.
    assert_eq!(
        std::fs::read_to_string(fake.project.join("out.txt")).unwrap(),
        "written\n"
    );
    assert!(!fake.home.join("scratch").exists());
    assert_eq!(
        std::fs::read_to_string(fake.home.join(".pi/agent/auth.json")).unwrap(),
        "{\"login\":\"pi\"}"
    );
}

/// As a chat another server starts runs: in a working copy in Zend's data,
/// on a key of its own. None of the owner's logins is there (though the
/// driver asks for them), the owner's project and the rest of Zend's data
/// are out of sight, and of the credentials only its key is in its
/// environment, whatever the driver set.
#[tokio::test]
async fn a_working_copy_on_its_own_keys_has_no_owner_login_and_no_owner_project() {
    let Some(bwrap) = bwrap() else { return };
    let fake = fake_home();
    let data = fake.home.join(".zend");
    let copies = data.join("peer-copies");
    let copy = copies.join("c");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::write(copy.join("README"), "the copy").unwrap();
    let mut host = host(&fake, bwrap);
    host.hidden.push(data.clone());
    let keys = [("ANTHROPIC_API_KEY".to_string(), "PEER-KEY".to_string())].into_iter().collect();
    let sandbox = Sandbox::working_copy(&copy, &copies, host).unwrap().with_keys(keys).unwrap();
    let pi_logins = logins(HarnessId::Pi, &fake.home, &|_| None);
    assert!(!pi_logins.is_empty());
    let launch = Launch::new(&sandbox, HarnessId::Pi, Path::new("/bin/sh"), &copy, &pi_logins).unwrap();
    let h = fake.home.display();
    let script = format!(
        r#"
        for f in {h}/.pi/agent/auth.json {h}/.pi/agent/models.json {h}/projects/a/README {h}/.zend/ipc-token; do
            if cat "$f" >/dev/null 2>&1; then echo "READABLE $f"; fi
        done
        cat ./README; echo
        echo changed > ./out.txt && echo "wrote copy"
        env | sort
        "#
    );
    let mut command = launch.command("/bin/sh");
    command
        .arg("-c")
        .arg(script)
        .env("ANTHROPIC_API_KEY", "OWNER-ANTHROPIC")
        .env("OPENAI_API_KEY", "OWNER-OPENAI")
        .env("PI_CODING_AGENT_DIR", fake.home.join(".pi/agent"))
        .env("PATH", "/usr/bin:/bin")
        .current_dir(&copy)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = launch.spawn(&mut command).unwrap().wait_with_output().await.unwrap();
    let out = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("exit {:?}\n{out}{}", output.status, String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success());
    assert!(!out.contains("READABLE"), "{out}");
    assert!(out.contains("the copy") && out.contains("wrote copy"), "{out}");
    let credentials: Vec<&str> = out
        .lines()
        .filter(|line| ["KEY", "TOKEN", "SECRET", "AUTH"].iter().any(|w| line.split('=').next().unwrap().contains(w)))
        .collect();
    assert_eq!(credentials, ["ANTHROPIC_API_KEY=PEER-KEY"], "{out}");
    assert!(!out.contains("OWNER-"), "{out}");
    assert_eq!(std::fs::read_to_string(copy.join("out.txt")).unwrap(), "changed\n");
    assert_eq!(std::fs::read_to_string(fake.project.join("README")).unwrap(), "project a");
}

#[tokio::test]
async fn a_sandboxed_agent_inherits_no_zend_or_ssh_variables() {
    let Some(bwrap) = bwrap() else { return };
    let fake = fake_home();
    let (code, out, err) = run_in(&fake, bwrap, "env | sort").await;
    println!("exit {code}\n--- env\n{out}--- stderr\n{err}");
    assert_eq!(code, 0, "{err}");
    for line in out.lines() {
        let key = line.split('=').next().unwrap();
        assert!(
            !key.starts_with("ZEND_")
                && !key.starts_with("SSH_")
                && key != "DBUS_SESSION_BUS_ADDRESS",
            "{line}"
        );
    }
    assert!(out.contains(&format!("HOME={}\n", fake.home.display())), "{out}");
    assert!(out.contains("PI_CODING_AGENT_DIR="), "{out}");
    assert!(out.lines().any(|line| line.starts_with("PATH=")), "{out}");
}

/// What the owner's own tools run or load from the project afterwards
/// (git's folder, agents' extensions, editor tasks) the agent can't change;
/// the rest of the project it can.
#[tokio::test]
async fn a_sandboxed_agent_cannot_change_git_or_agent_settings_in_its_project() {
    let Some(bwrap) = bwrap() else { return };
    let fake = fake_home();
    let p = &fake.project;
    let write = |path: &str, text: &str| {
        let path = p.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(".git/config", "[core]\n");
    write(".git/hooks/README", "hooks");
    write(".git/worktrees/w/HEAD", "ref: refs/heads/w\n");
    write(".pi/extensions/keep.ts", "// owner's");
    write(".vscode/tasks.json", "{}");
    write(".envrc", "use nix");
    // A worktree inside the project: its .git file names its git folder.
    write("w/.git", &format!("gitdir: {}\n", p.join(".git/worktrees/w").display()));
    write("AGENTS.md", "owner's");
    let script = r#"
        try() { if (eval "$1") 2>/dev/null; then echo "CHANGED $1"; fi; }
        try 'printf "[core]\n fsmonitor = evil\n" >> .git/config'
        try 'echo x > .git/hooks/pre-commit'
        try 'echo x > .git/worktrees/w/HEAD'
        try 'echo x > .pi/extensions/evil.ts'
        try 'rm .pi/extensions/keep.ts'
        try 'echo x > .vscode/tasks.json'
        try 'echo x >> .envrc'
        try 'mv .git .git-moved'
        try 'mv .pi .pi-moved'
        echo agent > AGENTS.md && echo "wrote AGENTS.md"
        echo code > src.rs && echo "wrote src.rs"
        cat .git/config >/dev/null && echo "git readable"
    "#;
    let (code, out, err) = run_in(&fake, bwrap, script).await;
    println!("exit {code}\n--- stdout\n{out}--- stderr\n{err}");
    assert_eq!(code, 0, "{err}");
    assert!(!out.contains("CHANGED"), "{out}");
    assert!(out.contains("wrote AGENTS.md") && out.contains("wrote src.rs") && out.contains("git readable"), "{out}");
    assert_eq!(std::fs::read_to_string(p.join(".git/config")).unwrap(), "[core]\n");
    assert!(!p.join(".pi/extensions/evil.ts").exists());
    assert!(p.join(".pi/extensions/keep.ts").exists());
    assert_eq!(std::fs::read_to_string(p.join("src.rs")).unwrap(), "code\n");
}

#[tokio::test]
async fn a_sandboxed_agent_runs_in_a_pid_namespace_of_its_own() {
    let Some(bwrap) = bwrap() else { return };
    let fake = fake_home();
    // Only bubblewrap's init and the shell's own processes: none of the host's.
    let (code, out, err) = run_in(&fake, bwrap, "ls /proc | grep -c '^[0-9]'").await;
    assert_eq!(code, 0, "{err}");
    let count: u32 = out.trim().parse().unwrap();
    assert!(count <= 5, "{count} processes visible");
}

#[test]
fn a_bubblewrap_that_cannot_start_refuses_the_run() {
    if bwrap().is_none() {
        return;
    }
    let fake = fake_home();
    // Stands in for a bwrap that can't make its namespaces: exits non-zero.
    let sandbox = Sandbox::project(&fake.project, host(&fake, PathBuf::from("/bin/false"))).unwrap();
    let error = Launch::new(&sandbox, HarnessId::Pi, Path::new("/bin/sh"), &fake.project, &[])
        .unwrap_err()
        .to_string();
    assert!(error.contains("must run sandboxed and was not started"), "{error}");
    assert!(error.contains("the sandbox failed to start"), "{error}");
    // A folder outside the project is refused before bubblewrap runs.
    let error = Launch::new(&sandbox, HarnessId::Pi, Path::new("/bin/sh"), &fake.sibling, &[])
        .unwrap_err()
        .to_string();
    assert!(error.contains("outside its project"), "{error}");
}

/// The real Pi of this device, signed in with its real login, shows its
/// version and lists its models from inside the sandbox (no prompt: no
/// model usage).
#[tokio::test]
#[ignore = "needs Pi signed in on this device and bwrap"]
async fn real_pi_lists_its_models_in_the_sandbox() {
    let Some(bwrap) = bwrap() else { return };
    let exe = zend_harness::PiHarness::new()
        .resolve_executable()
        .expect("Pi installed");
    let host = Host {
        bwrap: Some(bwrap),
        ..Host::current()
    };
    // A project inside the home, as every sandboxed root must be.
    let project = tempfile::Builder::new()
        .prefix("zend-sandbox-test-")
        .tempdir_in(&host.home)
        .unwrap();
    let sandbox = Sandbox::project(project.path(), host).unwrap();
    let home = sandbox.host().home.clone();
    let pi_logins = logins(HarnessId::Pi, &home, &|key| std::env::var_os(key));
    let launch = Launch::new(&sandbox, HarnessId::Pi, &exe, project.path(), &pi_logins).unwrap();
    println!("logins {pi_logins:?}\nbwrap {:?}", launch.bwrap_args());
    for args in [&["--version"][..], &["--list-models"][..]] {
        let mut command = launch.command(&exe);
        command
            .args(args)
            .current_dir(project.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = launch
            .spawn(&mut command)
            .unwrap()
            .wait_with_output()
            .await
            .unwrap();
        let out = String::from_utf8_lossy(&output.stdout);
        let err = String::from_utf8_lossy(&output.stderr);
        println!("$ pi {}\nexit {}\n{out}{err}", args.join(" "), output.status);
        assert!(output.status.success(), "{err}");
    }
    // The network stays on: the agent reaches its provider's API by name.
    let mut command = launch.command("/bin/sh");
    command
        .arg("-c")
        .arg("getent hosts api.anthropic.com && curl -sS -o /dev/null -w 'https %{http_code}\\n' https://api.anthropic.com/")
        .current_dir(project.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = launch
        .spawn(&mut command)
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
    let out = String::from_utf8_lossy(&output.stdout);
    println!("$ network\nexit {}\n{out}{}", output.status, String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success());
    assert!(out.contains("https "), "{out}");
}

/// What the host would reach from outside: a TCP listener on every address
/// and an abstract unix socket (the kind X11 and D-Bus listen on).
struct Listeners {
    port: u16,
    abstract_name: String,
    _tcp: std::net::TcpListener,
    _unix: std::os::unix::net::UnixListener,
}

fn listeners() -> Listeners {
    use std::os::linux::net::SocketAddrExt;
    let tcp = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let port = tcp.local_addr().unwrap().port();
    let abstract_name = format!("zend-sandbox-test-{}-{port}", std::process::id());
    let addr = std::os::unix::net::SocketAddr::from_abstract_name(&abstract_name).unwrap();
    let unix = std::os::unix::net::UnixListener::bind_addr(&addr).unwrap();
    Listeners { port, abstract_name, _tcp: tcp, _unix: unix }
}

/// A shell script that prints "REACHED <what>" for each of `targets` (host
/// and port) it can connect to, and for the abstract socket `name`.
fn probe_script(targets: &[(String, u16)], name: &str) -> String {
    let mut script = String::new();
    for (host, port) in targets {
        script.push_str(&format!(
            "nc -z -w 2 {host} {port} 2>/dev/null && echo 'REACHED {host}:{port}'\n"
        ));
    }
    script.push_str(&format!(
        "python3 -c \"import socket; socket.socket(socket.AF_UNIX).connect('\\0{name}')\" 2>/dev/null && echo 'REACHED @{name}'\n"
    ));
    script
}

#[tokio::test]
async fn a_sandboxed_agent_cannot_reach_the_host_its_networks_or_its_sockets() {
    let Some(bwrap) = bwrap() else { return };
    let fake = fake_home();
    let listen = listeners();
    let mut targets = vec![("127.0.0.1".to_string(), listen.port)];
    // The host's own addresses (its public one, the tailnet's, docker's).
    for net in zend_harness::sandbox::host_networks() {
        if let Some(addr) = net.strip_suffix("/32") {
            targets.push((addr.to_string(), listen.port));
        }
    }
    // Tailscale's resolver, a cloud's metadata service.
    targets.push(("100.100.100.100".into(), 53));
    targets.push(("169.254.169.254".into(), 80));
    let script = probe_script(&targets, &listen.abstract_name);
    // From outside, the host's listeners answer: the probe itself works.
    let outside = std::process::Command::new("/bin/sh").arg("-c").arg(&script).output().unwrap();
    let outside = String::from_utf8_lossy(&outside.stdout).into_owned();
    println!("--- outside\n{outside}");
    assert!(outside.contains(&format!("REACHED 127.0.0.1:{}", listen.port)), "{outside}");
    assert!(outside.contains(&format!("REACHED @{}", listen.abstract_name)), "{outside}");
    let inside = format!(
        "{script}ip -4 route del unreachable 10.0.0.0/8 2>/dev/null && echo 'ROUTE CHANGED'\nip -4 route\n"
    );
    let (code, out, err) = run_in(&fake, bwrap, &inside).await;
    println!("exit {code}\n--- inside\n{out}--- stderr\n{err}");
    assert_eq!(code, 0, "{err}");
    assert!(!out.contains("REACHED"), "{out}");
    assert!(!out.contains("ROUTE CHANGED"), "{out}");
    assert!(out.contains("unreachable 100.64.0.0/10"), "{out}");
}

/// Outbound internet still works in the sandbox, by name: DNS through
/// pasta's forwarder, then HTTPS.
#[tokio::test]
#[ignore = "needs the internet, bwrap and pasta"]
async fn a_sandboxed_agent_reaches_the_internet_by_name() {
    let Some(bwrap) = bwrap() else { return };
    let fake = fake_home();
    let (code, out, err) = run_in(
        &fake,
        bwrap,
        "cat /etc/resolv.conf; getent hosts api.anthropic.com && curl -sS -o /dev/null -w 'https %{http_code}\\n' https://api.anthropic.com/",
    )
    .await;
    println!("exit {code}\n{out}--- stderr\n{err}");
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("nameserver 169.254.2.1"), "{out}");
    assert!(out.contains("https "), "{out}");
}

#[tokio::test]
async fn an_agent_in_the_shared_bin_folder_brings_only_its_own_file() {
    let Some(bwrap) = bwrap() else { return };
    let fake = fake_home();
    let write = |path: &Path, text: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let bin = fake.home.join(".local/bin");
    write(&bin.join("owner-script"), "#!/bin/sh\necho OWNER SECRET\n");
    // An agent linked from ~/.local/bin to its install, and one that lives there.
    write(&fake.home.join("tools/agent/agent"), "#!/bin/sh\necho agent ok\n");
    std::os::unix::fs::symlink(fake.home.join("tools/agent/agent"), bin.join("agent")).unwrap();
    write(&bin.join("direct"), "#!/bin/sh\necho direct ok\n");
    let script = format!(
        "ls {b}; {b}/agent; cat {b}/owner-script 2>/dev/null && echo 'OWNER READABLE'; true",
        b = bin.display()
    );
    let (code, out, err) = run_as(&fake, bwrap.clone(), &bin.join("agent"), &script).await;
    println!("exit {code}\n{out}--- stderr\n{err}");
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("agent ok"), "{out}");
    assert!(!out.contains("owner-script") && !out.contains("OWNER"), "{out}");
    let (code, out, err) = run_as(&fake, bwrap, &bin.join("direct"), &format!("ls {}; {}/direct", bin.display(), bin.display())).await;
    println!("exit {code}\n{out}--- stderr\n{err}");
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("direct ok"), "{out}");
    assert!(!out.contains("owner-script"), "{out}");
}

#[test]
fn a_tool_folder_that_would_show_the_home_or_a_hidden_folder_is_refused() {
    let Some(bwrap) = bwrap() else { return };
    for (dir, target) in [(".cargo/bin", ""), (".rustup", ".ssh"), (".volta", "projects")] {
        let fake = fake_home();
        let link = fake.home.join(dir);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(fake.home.join(target), &link).unwrap();
        let sandbox = Sandbox::project(&fake.project, host(&fake, bwrap.clone())).unwrap();
        let error = Launch::new(&sandbox, HarnessId::Pi, Path::new("/bin/sh"), &fake.project, &[])
            .unwrap_err()
            .to_string();
        println!("{dir} -> ~/{target}: {error}");
        assert!(error.contains("would show"), "{dir}: {error}");
    }
}


// ---- Node launchers: pnpm's shim, npm's link, a plain binary ----

fn write_exe(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn link(target: impl AsRef<Path>, at: &Path) {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, at).unwrap();
}

/// The shim pnpm 11 writes (as on a real device), for `rel` from its bin.
fn pnpm_shim(rel: &str, abs: &Path) -> String {
    let body = r#"#!/bin/sh
basedir=$(dirname "$(echo "$0" | sed -e 's,\\,/,g')")
basedir_win="$basedir"
exe=""
msys=""

case `uname -a` in
  *CYGWIN*|*MINGW*|*MSYS*)
    if command -v cygpath > /dev/null 2>&1; then
      basedir_win=`cygpath -w "$basedir"`
    fi
    exe=".exe"
    msys="true"
  ;;
  *WSL2*)
    if command -v wslpath > /dev/null 2>&1; then
      basedir_win="$(wslpath -w "$basedir" 2> /dev/null)"
      if [ $? -ne 0 ] || [ -z "$basedir_win" ]; then
        basedir_win="$basedir"
      else
        exe=".exe"
      fi
    fi
  ;;
esac

if [ -n "$exe" ] && [ -x "$basedir/node.exe" ]; then
  exec "$basedir/node.exe"  "$basedir_win/REL" "$@"
elif [ -x "$basedir/node" ]; then
  exec "$basedir/node"  "$basedir/REL" "$@"
elif command -v node >/dev/null 2>&1; then
  exec node  "$basedir/REL" "$@"
elif [ -n "$exe" ] && command -v node.exe >/dev/null 2>&1; then
  exec node.exe  "$basedir_win/REL" "$@"
else
  exec node  "$basedir/REL" "$@"
fi
# cmd-shim-target=ABS
"#;
    body.replace("REL", rel).replace("ABS", &abs.display().to_string())
}

/// The agent's script: what it loads, which of `argv` it can see (`w:<path>`:
/// can write), and a file written in its project.
const AGENT_JS: &str = r#"#!/usr/bin/env node
const fs = require('fs');
console.log('loaded ' + require('dep-a'));
for (const a of process.argv.slice(2)) {
  if (a.startsWith('w:')) { try { fs.writeFileSync(a.slice(2), 'x'); console.log('WRITABLE ' + a.slice(2)); } catch {} }
  else if (fs.existsSync(a)) console.log('VISIBLE ' + a);
}
fs.writeFileSync('./from-agent.txt', 'ok');
"#;

/// `w:<path>` probes for files in each of `dirs`.
fn write_probes(dirs: &[&Path]) -> Vec<PathBuf> {
    dirs.iter().map(|dir| PathBuf::from(format!("w:{}", dir.join("written-by-agent").display()))).collect()
}

/// Run `exe` as the agent of a launch in the fake home's project a, with
/// `args`; (exit code, stdout, stderr, bubblewrap's arguments).
async fn run_agent(fake: &FakeHome, bwrap: PathBuf, exe: &Path, args: &[PathBuf]) -> (i32, String, String, Vec<String>) {
    let sandbox = Sandbox::project(&fake.project, host(fake, bwrap)).unwrap();
    let launch = Launch::new(&sandbox, HarnessId::Pi, exe, &fake.project, &[]).unwrap();
    let mut command = launch.command(exe);
    command
        .args(args)
        .env("PATH", "/usr/bin:/bin")
        .current_dir(&fake.project)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = launch.spawn(&mut command).unwrap().wait_with_output().await.unwrap();
    let bwrap_args = launch.bwrap_args().iter().map(|a| a.to_string_lossy().into_owned()).collect();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        bwrap_args,
    )
}

/// Every (flag, source, target) of bubblewrap's arguments whose source or
/// target lies in `under`.
fn mounts_in(args: &[String], under: &Path) -> Vec<(String, String, String)> {
    let under = under.display().to_string();
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        if matches!(flag, "--ro-bind" | "--bind" | "--symlink" | "--ro-bind-try" | "--bind-try" | "--dev-bind") && i + 2 < args.len() {
            if args[i + 1].starts_with(&under) || args[i + 2].starts_with(&under) {
                out.push((flag.to_string(), args[i + 1].clone(), args[i + 2].clone()));
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    out
}

fn node_installed() -> bool {
    let found = Path::new("/usr/bin/node").is_file();
    if !found {
        eprintln!("no /usr/bin/node: skipped");
    }
    found
}

/// pnpm's layout, outside the sandbox's home (another PNPM_HOME, or an
/// engine with a home of its own): bin/pi a shim, bin/node a link into the
/// store, the package a link from global/ into the store, its dependencies
/// links beside it into the store. It runs: node on its script, with
/// exactly its packages and node read-only, nothing else of pnpm's.
#[tokio::test]
async fn a_cli_installed_by_pnpm_runs_with_only_its_node_and_its_packages() {
    let Some(bwrap) = bwrap() else { return };
    if !node_installed() {
        return;
    }
    let fake = fake_home();
    let outside = tempfile::tempdir().unwrap();
    let pnpm = std::fs::canonicalize(outside.path()).unwrap().join("pnpm");
    let links = pnpm.join("store/v11/links");
    let node = links.join("@/node/26.0.0/h0/node_modules/node/bin/node");
    write_exe(&node, "#!/bin/sh\nexec /usr/bin/node \"$@\"\n");
    link(&node, &pnpm.join("bin/node"));
    let agent = links.join("@s/agent/1.0.0/ha/node_modules/@s/agent");
    write_exe(&agent.join("package.json"), r#"{"name":"@s/agent","bin":{"pi":"dist/cli.js"},"dependencies":{"dep-a":"1"}}"#);
    write_exe(&agent.join("dist/cli.js"), AGENT_JS);
    let dep_a = links.join("@/dep-a/1.0.0/hb/node_modules/dep-a");
    write_exe(&dep_a.join("package.json"), r#"{"name":"dep-a","dependencies":{"dep-b":"1"}}"#);
    write_exe(&dep_a.join("index.js"), "module.exports = 'a+' + require('dep-b');\n");
    let dep_b = links.join("@/dep-b/1.0.0/hc/node_modules/dep-b");
    write_exe(&dep_b.join("package.json"), r#"{"name":"dep-b"}"#);
    write_exe(&dep_b.join("index.js"), "module.exports = 'b';\n");
    link("../../../../../@/dep-a/1.0.0/hb/node_modules/dep-a", &links.join("@s/agent/1.0.0/ha/node_modules/dep-a"));
    link("../../../../dep-b/1.0.0/hc/node_modules/dep-b", &links.join("@/dep-a/1.0.0/hb/node_modules/dep-b"));
    let secret = links.join("@/secret/1.0.0/hs/node_modules/secret/index.js");
    write_exe(&secret, "secret");
    link("../../../../../store/v11/links/@s/agent/1.0.0/ha/node_modules/@s/agent", &pnpm.join("global/v11/g1/node_modules/@s/agent"));
    write_exe(&pnpm.join("global/v11/g1/package.json"), "{}");
    write_exe(&pnpm.join("bin/other"), "#!/bin/sh\n");
    let rel = "../global/v11/g1/node_modules/@s/agent/dist/cli.js";
    write_exe(&pnpm.join("bin/pi"), &pnpm_shim(rel, &pnpm.join("global/v11/g1/node_modules/@s/agent/dist/cli.js")));
    let mut probes = vec![secret.clone(), pnpm.join("bin/other"), pnpm.join("global/v11/g1/package.json"), links.join("@/secret")];
    // Node's folder is the sandbox's own (only its file is bound): try
    // the file itself.
    probes.extend(write_probes(&[&agent, &dep_a, &dep_b]));
    probes.push(PathBuf::from(format!("w:{}", node.display())));
    let (code, out, err, args) = run_agent(&fake, bwrap, &pnpm.join("bin/pi"), &probes).await;
    for dir in [&agent, &dep_a, &dep_b] {
        assert!(!dir.join("written-by-agent").exists(), "{}", dir.display());
    }
    assert!(std::fs::read_to_string(&node).unwrap().contains("/usr/bin/node"));
    let mounts = mounts_in(&args, &pnpm);
    println!("exit {code}\n--- stdout\n{out}--- stderr\n{err}--- mounts\n{mounts:#?}");
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("loaded a+b"), "{out}");
    assert!(!out.contains("VISIBLE") && !out.contains("WRITABLE"), "{out}");
    assert_eq!(std::fs::read_to_string(fake.project.join("from-agent.txt")).unwrap(), "ok");
    // Read-only: node's real file and the three packages, each at its real
    // path; the two dependency links as links; nothing writable.
    let path = |p: &Path| p.display().to_string();
    let mut binds: Vec<String> = mounts.iter().filter(|m| m.0 == "--ro-bind").map(|m| { assert_eq!(m.1, m.2); m.1.clone() }).collect();
    binds.sort();
    let mut expected = vec![path(&node), path(&agent), path(&dep_a), path(&dep_b)];
    expected.sort();
    assert_eq!(binds, expected);
    let symlinks: Vec<&String> = mounts.iter().filter(|m| m.0 == "--symlink").map(|m| &m.2).collect();
    assert_eq!(symlinks.len(), 2, "{mounts:?}");
    assert!(mounts.iter().all(|m| m.0 == "--ro-bind" || m.0 == "--symlink"), "{mounts:?}");
}

/// npm's layout, outside the sandbox's home: bin/agent a link to the
/// package's `#!/usr/bin/env node` script, its dependencies inside the
/// package. The package is there whole; npm's other packages are not.
#[tokio::test]
async fn a_cli_installed_by_npm_runs_with_its_package_and_not_its_neighbours() {
    let Some(bwrap) = bwrap() else { return };
    if !node_installed() {
        return;
    }
    let fake = fake_home();
    let outside = tempfile::tempdir().unwrap();
    let prefix = std::fs::canonicalize(outside.path()).unwrap().join("npm-global");
    let agent = prefix.join("lib/node_modules/@s/agent");
    write_exe(&agent.join("package.json"), r#"{"name":"@s/agent","dependencies":{"dep-a":"1"}}"#);
    write_exe(&agent.join("dist/cli.js"), AGENT_JS);
    write_exe(&agent.join("node_modules/dep-a/index.js"), "module.exports = 'npm-a';\n");
    let other = prefix.join("lib/node_modules/other/secret.js");
    write_exe(&other, "secret");
    link("../lib/node_modules/@s/agent/dist/cli.js", &prefix.join("bin/agent"));
    write_exe(&prefix.join("bin/other"), "#!/bin/sh\n");
    let mut probes = vec![other.clone(), prefix.join("bin/other")];
    probes.extend(write_probes(&[&agent]));
    let (code, out, err, args) = run_agent(&fake, bwrap, &prefix.join("bin/agent"), &probes).await;
    let mounts = mounts_in(&args, &prefix);
    println!("exit {code}\n--- stdout\n{out}--- stderr\n{err}--- mounts\n{mounts:#?}");
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("loaded npm-a"), "{out}");
    assert!(!out.contains("VISIBLE") && !out.contains("WRITABLE"), "{out}");
    assert_eq!(mounts, vec![("--ro-bind".to_string(), agent.display().to_string(), agent.display().to_string())]);
}

/// A plain binary outside the home runs as before: itself, its folder
/// read-only.
#[tokio::test]
async fn a_plain_binary_agent_runs_as_it_is_with_its_folder() {
    let Some(bwrap) = bwrap() else { return };
    let fake = fake_home();
    let outside = tempfile::tempdir().unwrap();
    let tools = std::fs::canonicalize(outside.path()).unwrap().join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::copy("/bin/echo", tools.join("agent")).unwrap();
    let (code, out, err, args) = run_agent(&fake, bwrap, &tools.join("agent"), &[PathBuf::from("plain ok")]).await;
    println!("exit {code}\n{out}{err}");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), "plain ok");
    let t = tools.display().to_string();
    assert_eq!(mounts_in(&args, &tools), vec![("--ro-bind".to_string(), t.clone(), t)]);
}

/// A pnpm shim that sets up more than node and its target is refused, not
/// run with more bound.
#[test]
fn a_pnpm_shim_that_does_more_is_refused() {
    let Some(bwrap) = bwrap() else { return };
    let fake = fake_home();
    let outside = tempfile::tempdir().unwrap();
    let pnpm = std::fs::canonicalize(outside.path()).unwrap().join("pnpm");
    write_exe(&pnpm.join("global/a/cli.js"), "#!/usr/bin/env node\n");
    write_exe(&pnpm.join("bin/node"), "#!/bin/sh\n");
    let shim = pnpm_shim("../global/a/cli.js", &pnpm.join("global/a/cli.js"))
        .replacen("exe=\"\"\n", "exe=\"\"\nexport NODE_PATH=\"/home\"\n", 1);
    write_exe(&pnpm.join("bin/pi"), &shim);
    let sandbox = Sandbox::project(&fake.project, host(&fake, bwrap)).unwrap();
    let error = Launch::new(&sandbox, HarnessId::Pi, &pnpm.join("bin/pi"), &fake.project, &[])
        .unwrap_err()
        .to_string();
    println!("{error}");
    assert!(error.contains("the pnpm shim") && error.contains("not run sandboxed"), "{error}");
}
