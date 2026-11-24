//! The bubblewrap sandbox, for real: what a confined process can see, write
//! and inherit. Needs `bwrap` (skipped, with a note, where it is missing).
#![cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
use std::process::Stdio;
use zend_harness::sandbox::{Host, Launch, Sandbox, logins};
use zend_proto::HarnessId;

fn bwrap() -> Option<PathBuf> {
    let path = PathBuf::from("/usr/bin/bwrap");
    if path.is_file() {
        return Some(path);
    }
    eprintln!("bwrap not installed: skipped");
    None
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
    let host = Host {
        home: fake.home.clone(),
        bwrap: Some(bwrap),
        hidden: Vec::new(),
    };
    let sandbox = Sandbox::project(&fake.project, host).unwrap();
    let pi_logins = logins(HarnessId::Pi, &fake.home, &|_| None);
    let launch = Launch::new(
        &sandbox,
        HarnessId::Pi,
        Path::new("/bin/sh"),
        &fake.project,
        &pi_logins,
    )
    .unwrap();
    let mut command = launch.command("/bin/sh");
    command
        .arg("-c")
        .arg(script)
        // What a driver would have set: never reaches the agent.
        .env("SSH_AUTH_SOCK", "/run/user/1000/keyring/ssh")
        .env("ZEND_IPC_TOKEN", "secret")
        .env("ZEND_ORIGIN", "chat:x")
        .env("PI_CODING_AGENT_DIR", fake.home.join(".pi/agent"))
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
    let fake = fake_home();
    // Stands in for a bwrap that can't make its namespaces: exits non-zero.
    let host = Host {
        home: fake.home.clone(),
        bwrap: Some(PathBuf::from("/bin/false")),
        hidden: Vec::new(),
    };
    let sandbox = Sandbox::project(&fake.project, host).unwrap();
    let error = Launch::new(&sandbox, HarnessId::Pi, Path::new("/bin/sh"), &fake.project, &[])
        .unwrap_err()
        .to_string();
    assert!(error.contains("must run sandboxed and was not started"), "{error}");
    assert!(error.contains("bubblewrap failed"), "{error}");
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
    let project = tempfile::tempdir().unwrap();
    let host = Host {
        bwrap: Some(bwrap),
        ..Host::current()
    };
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
}
