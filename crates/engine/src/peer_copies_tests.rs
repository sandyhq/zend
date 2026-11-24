//! Unit tests of [`super`]: the working copies of servers' chats.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

struct Rig {
    _temp: tempfile::TempDir,
    data: PathBuf,
    owner: PathBuf,
    copies: Copies,
    root: PathBuf,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn owner_folder(temp: &Path, repo: bool) -> PathBuf {
    let owner = temp.join("p");
    std::fs::create_dir_all(&owner).unwrap();
    std::fs::write(owner.join("readme.md"), "base\n").unwrap();
    std::fs::write(owner.join("run.sh"), "echo hi\n").unwrap();
    if repo {
        git(&owner, &["init", "-q", "-b", "main"]);
        git(&owner, &["config", "user.email", "o@example.com"]);
        git(&owner, &["config", "user.name", "Owner"]);
        git(&owner, &["add", "-A"]);
        git(&owner, &["commit", "-q", "-m", "base"]);
    }
    owner
}

fn rig(repo: bool) -> Rig {
    let temp = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(temp.path()).unwrap();
    let owner = owner_folder(&base, repo);
    let data = base.join("data");
    let copies = Copies::new(&data);
    let root = copies.create("c1", "peer", "p", &owner).expect("create");
    Rig { _temp: temp, data, owner, copies, root }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// Review item 1: what is applied is exactly the tree the owner was shown;
/// a copy that moved since is refused, and nothing of it applied.
#[test]
fn apply_writes_exactly_the_tree_the_owner_was_shown() {
    let r = rig(true);
    std::fs::write(r.root.join("readme.md"), "one\n").unwrap();
    let (shown, changes, patch) = r.copies.patch("c1").unwrap();
    assert!(patch.contains("+one") && changes.len() == 1, "{patch}");
    // The agent writes again after the owner read the patch.
    std::fs::write(r.root.join("readme.md"), "two, unread\n").unwrap();
    std::fs::write(r.root.join("extra.txt"), "unread\n").unwrap();
    let refused = r.copies.apply("c1", &shown, &[]).unwrap_err();
    assert!(refused.contains("changed since you were shown") && refused.contains("nothing applied"), "{refused}");
    assert_eq!(read(&r.owner.join("readme.md")), "base\n");
    assert!(!r.owner.join("extra.txt").exists());
    // Shown again, nothing changes meanwhile: that tree applies.
    let (shown, _, _) = r.copies.patch("c1").unwrap();
    let (_, listed) = r.copies.list().remove(0);
    assert_eq!(listed.unwrap().0, shown, "list names the same tree");
    let applied = r.copies.apply("c1", &shown, &[]).unwrap();
    assert_eq!(applied, ["extra.txt", "readme.md"]);
    assert_eq!(read(&r.owner.join("readme.md")), "two, unread\n");
    // A made-up tree: refused.
    assert!(r.copies.apply("c1", "0000000000000000000000000000000000000000", &[]).unwrap_err().contains("changed since"));
}

/// Review item 1: no list, show or apply while a run of a chat in the copy
/// is live (the sessions engine tells, [`on_held`]).
#[test]
fn a_copy_isnt_read_or_applied_while_a_run_in_it_is_live() {
    let r = rig(false);
    let live = Arc::new(AtomicBool::new(true));
    let held = {
        let (live, root) = (live.clone(), r.root.clone());
        move |at: &Path| (live.load(Ordering::SeqCst) && at == root).then(|| "c1-child".to_string())
    };
    on_held(&r.data, held);
    std::fs::write(r.root.join("readme.md"), "mid-run\n").unwrap();
    let refused = r.copies.patch("c1").unwrap_err();
    assert!(refused.contains("chat c1-child has a live run") && refused.contains("stop it, or wait"), "{refused}");
    assert!(r.copies.list()[0].1.as_ref().is_err_and(|e| e.contains("live run")));
    assert!(r.copies.apply("c1", "x", &[]).unwrap_err().contains("live run"));
    assert_eq!(read(&r.owner.join("readme.md")), "base\n");
    assert!(!reviewing(&r.root), "the mark goes with the refusal");
    live.store(false, Ordering::SeqCst);
    let (tree, _, _) = r.copies.patch("c1").unwrap();
    r.copies.apply("c1", &tree, &[]).unwrap();
    assert_eq!(read(&r.owner.join("readme.md")), "mid-run\n");
}

/// Review item 3: nor while a process of a sandbox of the copy lives (one
/// left behind by a run): found by its mount namespace holding the copy.
#[cfg(target_os = "linux")]
#[test]
fn a_copy_isnt_read_while_a_process_of_its_sandbox_lives() {
    let r = rig(false);
    let root = r.root.display().to_string();
    let spawned = std::process::Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--bind", &root, &root, "--unshare-pid", "--die-with-parent", "sleep", "60"])
        .stdin(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else {
        eprintln!("no bwrap: skipped");
        return;
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while sandbox_process(&r.root).is_none() {
        if std::time::Instant::now() > deadline || child.try_wait().unwrap().is_some() {
            let _ = child.kill();
            eprintln!("bwrap didn't start here: skipped");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::fs::write(r.root.join("readme.md"), "while it lives\n").unwrap();
    let refused = r.copies.patch("c1").unwrap_err();
    assert!(refused.contains("a process of this working copy's sandbox still runs"), "{refused}");
    // Another copy's sandbox isn't this one's.
    assert_eq!(sandbox_process(&r.data.join("peer-copies/other")), None);
    child.kill().unwrap();
    child.wait().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while sandbox_process(&r.root).is_some() {
        assert!(std::time::Instant::now() < deadline, "the sandbox's namespace stayed");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (_, changes, _) = r.copies.patch("c1").unwrap();
    assert_eq!(changes.len(), 1);
}

/// Review item 2: files the owner's tools run on their own are never
/// applied; files that instruct the owner's agents only path by path.
#[test]
fn files_that_run_for_the_owner_are_refused_and_instructions_need_an_allow() {
    let r = rig(true);
    let refused = [
        ".pre-commit-config.yaml", "lefthook.yml", ".lefthook.yml", ".lefthook/pre-commit/x.sh", "flake.nix", "flake.lock",
        "shell.nix", ".mcp.json", "opencode.json", ".zed/settings.json", ".devcontainer/devcontainer.json",
        ".github/workflows/ci.yml", "sub/.GitHub/Workflows/x.yml", "sub/.gitattributes",
    ];
    let flagged = ["AGENTS.md", "sub/CLAUDE.md", "GEMINI.md", ".cursorrules", ".windsurfrules"];
    let plain = ["default.nix", ".gitlab-ci.yml", "Makefile", ".github/CODEOWNERS", "docs/agents.md.txt"];
    for path in refused.iter().chain(&flagged).chain(&plain) {
        let file = r.root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, format!("{path}\n")).unwrap();
    }
    let (tree, changes, _) = r.copies.patch("c1").unwrap();
    for change in &changes {
        let path = change.path.as_str();
        match () {
            _ if refused.contains(&path) => assert!(change.refused.is_some() && change.flag.is_none(), "{change:?}"),
            _ if flagged.contains(&path) => assert_eq!((change.refused.as_deref(), change.flag.as_deref()), (None, Some(INSTRUCTS_NOTE)), "{change:?}"),
            _ => assert!(plain.contains(&path) && change.refused.is_none() && change.flag.is_none(), "{change:?}"),
        }
    }
    assert_eq!(changes.len(), refused.len() + flagged.len() + plain.len());
    // A path under .cursor is refused outright (a protected name).
    assert!(path_rule(".cursor/rules/x.mdc").is_err());
    let all: Vec<String> = flagged.iter().map(|p| p.to_string()).collect();
    let error = r.copies.apply("c1", &tree, &all).unwrap_err();
    for path in refused {
        assert!(error.contains(path), "{path}: {error}");
    }
    assert!(!r.owner.join("Makefile").exists(), "nothing applied");
    for path in refused {
        std::fs::remove_file(r.root.join(path)).unwrap();
    }
    let (tree, _, _) = r.copies.patch("c1").unwrap();
    // Not allowed, or only some: nothing applied, each named with its --allow.
    let error = r.copies.apply("c1", &tree, &[]).unwrap_err();
    assert!(error.contains("AGENTS.md: instructs or configures your tools") && error.contains("--allow sub/CLAUDE.md"), "{error}");
    let error = r.copies.apply("c1", &tree, &["AGENTS.md".into()]).unwrap_err();
    assert!(!error.contains("AGENTS.md:") && error.contains("sub/CLAUDE.md"), "{error}");
    assert!(!r.owner.join("AGENTS.md").exists());
    let applied = r.copies.apply("c1", &tree, &all).unwrap();
    assert_eq!(applied.len(), flagged.len() + plain.len(), "{applied:?}");
    assert_eq!(read(&r.owner.join("sub/CLAUDE.md")), "sub/CLAUDE.md\n");
    assert_eq!(read(&r.owner.join("Makefile")), "Makefile\n");
}

/// Review item 4: a copy too large is refused before git reads it, naming
/// what is too large; sizes are apparent; what .gitignore leaves out isn't
/// read, so isn't counted.
#[test]
fn a_copy_too_large_to_read_is_refused_by_name() {
    let r = rig(false);
    let sparse = |name: &str, len: u64| {
        let file = std::fs::File::create(r.root.join(name)).unwrap();
        file.set_len(len).unwrap();
    };
    sparse("big.bin", MAX_FILE + 1);
    let error = r.copies.patch("c1").unwrap_err();
    assert!(error.contains("larger than 20.0 MiB") && error.contains("big.bin (20.0 MiB)"), "{error}");
    std::fs::remove_file(r.root.join("big.bin")).unwrap();
    for i in 0..26 {
        sparse(&format!("part{i:02}.bin"), MAX_FILE - 1);
    }
    let error = r.copies.patch("c1").unwrap_err();
    assert!(error.contains("more than 500.0 MiB") && error.contains("part"), "{error}");
    std::fs::create_dir(r.root.join("target")).unwrap();
    for i in 0..26 {
        std::fs::rename(r.root.join(format!("part{i:02}.bin")), r.root.join(format!("target/part{i:02}.bin"))).unwrap();
    }
    std::fs::write(r.root.join(".gitignore"), "target/\n").unwrap();
    let (_, changes, _) = r.copies.patch("c1").expect("what .gitignore leaves out isn't read");
    assert_eq!(changes.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), [".gitignore"]);
    std::fs::create_dir(r.root.join("many")).unwrap();
    for i in 0..MAX_ENTRIES {
        std::fs::write(r.root.join(format!("many/{i}")), "").unwrap();
    }
    let error = r.copies.patch("c1").unwrap_err();
    assert!(error.contains("more than 50000 files"), "{error}");
    assert!(r.copies.apply("c1", "x", &[]).unwrap_err().contains("more than 50000 files"));
}

/// Review item 9: a change of mode only (executable or not) is applied,
/// both ways, and then is the base.
#[cfg(unix)]
#[test]
fn a_mode_only_change_is_applied() {
    use std::os::unix::fs::PermissionsExt as _;
    let r = rig(true);
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&r.owner.join("run.sh")) & 0o111, 0);
    std::fs::set_permissions(r.root.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let (tree, changes, _) = r.copies.patch("c1").unwrap();
    assert_eq!(changes.len(), 1);
    assert!(changes[0].mode_only && changes[0].refused.is_none(), "{changes:?}");
    assert_eq!(r.copies.apply("c1", &tree, &[]).unwrap(), ["run.sh"]);
    assert_eq!(mode(&r.owner.join("run.sh")) & 0o111, 0o111);
    assert_eq!(read(&r.owner.join("run.sh")), "echo hi\n");
    assert!(r.copies.patch("c1").unwrap().1.is_empty(), "the base moved with it");
    std::fs::set_permissions(r.root.join("run.sh"), std::fs::Permissions::from_mode(0o644)).unwrap();
    let (tree, _, _) = r.copies.patch("c1").unwrap();
    r.copies.apply("c1", &tree, &[]).unwrap();
    assert_eq!(mode(&r.owner.join("run.sh")) & 0o111, 0);
    // The owner made it executable meanwhile, the copy didn't: changed in
    // your folder, refused.
    std::fs::set_permissions(r.root.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(r.owner.join("run.sh"), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(r.owner.join("run.sh"), "echo mine\n").unwrap();
    let (tree, _, _) = r.copies.patch("c1").unwrap();
    assert!(r.copies.apply("c1", &tree, &[]).unwrap_err().contains("changed in your folder"));
}

/// Review item 7: the worktree is detached (no branch of it in the owner's
/// repository) and making and discarding it runs none of the owner's hooks.
#[cfg(unix)]
#[test]
fn a_worktree_is_detached_and_runs_none_of_the_owners_hooks() {
    use std::os::unix::fs::PermissionsExt as _;
    let temp = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(temp.path()).unwrap();
    let owner = owner_folder(&base, true);
    let marker = base.join("HOOK-RAN");
    let hooks = owner.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    for hook in ["reference-transaction", "post-checkout", "post-index-change", "post-rewrite"] {
        let file = hooks.join(hook);
        std::fs::write(&file, format!("#!/bin/sh\necho {hook} >> {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // Also where the owner's config points hooks.
    git(&owner, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let copies = Copies::new(&base.join("data"));
    let root = copies.create("c1", "peer", "p", &owner).expect("create");
    assert!(!marker.exists(), "a hook ran: {}", std::fs::read_to_string(&marker).unwrap_or_default());
    assert_eq!(git(&owner, &["for-each-ref", "--format=%(refname)"]).trim(), "refs/heads/main");
    assert_eq!(copies.meta("c1").unwrap().branch, None);
    let head = std::fs::read_to_string(owner.join(".git/worktrees/c1/HEAD")).unwrap();
    assert!(!head.starts_with("ref:"), "detached: {head}");
    assert!(root.join("readme.md").is_file());
    copies.discard("c1").unwrap();
    assert!(!marker.exists(), "a hook ran: {}", std::fs::read_to_string(&marker).unwrap_or_default());
    assert!(!owner.join(".git/worktrees/c1").exists(), "pruned");
    // Control: the hooks are live for the owner's own git.
    git(&owner, &["checkout", "-q", "-b", "other"]);
    assert!(marker.exists(), "the control hook didn't run");
}

/// Review item 8: a plain copy leaves out the files that hold secrets by
/// their name, and Zend's data (here inside the owner's folder).
#[test]
fn a_plain_copy_leaves_secrets_and_zends_data_out() {
    let temp = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(temp.path()).unwrap();
    let owner = owner_folder(&base, false);
    let secrets = [".env", ".env.local", ".netrc", ".npmrc", ".pypirc", "cert.pem", "server.key", "id_ed25519", "sub/.env", "sub/id_rsa.pub"];
    let kept = ["readme.md", "env.txt", "key.txt", "sub/notes.md"];
    for path in secrets.iter().chain(&kept) {
        let file = owner.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "x\n").unwrap();
    }
    // Zend's data in the project: never copied (nor would the copy end).
    let copies = Copies::new(&owner.join("zend-data"));
    let root = copies.create("c1", "peer", "p", &owner).expect("create");
    for path in secrets {
        assert!(!root.join(path).exists(), "{path} was copied");
    }
    for path in kept {
        assert!(root.join(path).is_file(), "{path} wasn't copied");
    }
    assert!(!root.join("zend-data").exists());
    // Left out isn't deleted: not a change to apply.
    let (_, changes, _) = copies.patch("c1").unwrap();
    assert!(changes.is_empty(), "{changes:?}");
}

/// Review item 8: a worktree has only what the owner committed: no
/// untracked or ignored file of the owner's folder.
#[test]
fn a_worktree_has_none_of_the_owners_untracked_or_ignored_files() {
    let temp = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(temp.path()).unwrap();
    let owner = owner_folder(&base, true);
    std::fs::write(owner.join(".gitignore"), ".env\nsecret.key\nbuild/\n").unwrap();
    git(&owner, &["add", ".gitignore"]);
    git(&owner, &["commit", "-q", "-m", "ignore"]);
    for path in [".env", "secret.key", "build/token", "notes.txt"] {
        let file = owner.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "SECRET\n").unwrap();
    }
    let copies = Copies::new(&base.join("data"));
    let root = copies.create("c1", "peer", "p", &owner).expect("create");
    for path in [".env", "secret.key", "build", "notes.txt"] {
        assert!(!root.join(path).exists(), "{path} is in the worktree");
    }
    assert!(root.join("readme.md").is_file() && root.join(".gitignore").is_file());
}
