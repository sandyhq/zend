use super::*;

fn project() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(tmp.path()).unwrap();
    let root = base.join("p");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), "a").unwrap();
    std::fs::write(base.join("secret"), "OUTSIDE").unwrap();
    let data = base.join("data");
    std::fs::create_dir_all(&data).unwrap();
    (tmp, root, data)
}

#[test]
fn a_path_stays_inside_its_project() {
    let (tmp, root, data) = project();
    let base = std::fs::canonicalize(tmp.path()).unwrap();
    assert_eq!(read(&root, "src/a.txt", &data).unwrap().text.as_deref(), Some("a"));
    assert_eq!(read(&root, "./src/a.txt", &data).unwrap().text.as_deref(), Some("a"));
    assert!(list(&root, "", &data).unwrap().iter().any(|e| e.name == "src" && e.kind == "dir"));
    assert!(list(&root, "./src", &data).unwrap().iter().any(|e| e.name == "a.txt" && e.size == Some(1)));
    let absolute = base.join("secret").to_string_lossy().into_owned();
    for bad in ["../secret", "src/../../secret", "/etc/passwd", absolute.as_str()] {
        let refused = read(&root, bad, &data).unwrap_err();
        assert!(refused.contains("from the project's root"), "{bad}: {refused}");
        assert!(list(&root, bad, &data).is_err(), "{bad}");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(base.join("secret"), root.join("out")).unwrap();
        std::os::unix::fs::symlink(&base, root.join("up")).unwrap();
        std::os::unix::fs::symlink("src/a.txt", root.join("in")).unwrap();
        // No link is followed: out of the project, or inside it.
        for link in ["out", "up/secret", "up", "in"] {
            let refused = read(&root, link, &data).unwrap_err();
            assert!(refused.contains("link") || refused.contains("not a file"), "{link}: {refused}");
        }
        assert!(list(&root, "up", &data).is_err());
        let listed = list(&root, "", &data).unwrap();
        assert!(listed.iter().any(|e| e.name == "out" && e.kind == "link"), "{listed:?}");
        // A hard link to a file outside: one more name, refused.
        std::fs::hard_link(base.join("secret"), root.join("hard")).unwrap();
        let refused = read(&root, "hard", &data).unwrap_err();
        assert!(refused.contains("hard link"), "{refused}");
    }
}

/// The fallback (no openat2) walks one name at a time and follows no link either.
#[cfg(unix)]
#[test]
fn the_component_walk_follows_no_link() {
    use rustix::fs::{Mode, OFlags};
    let (tmp, root, _data) = project();
    let base = std::fs::canonicalize(tmp.path()).unwrap();
    std::os::unix::fs::symlink(&base, root.join("up")).unwrap();
    std::os::unix::fs::symlink(base.join("secret"), root.join("out")).unwrap();
    let dir = rustix::fs::open(&root, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty()).unwrap();
    assert!(imp::walk(&dir, Path::new("src/a.txt"), OFlags::RDONLY).is_ok());
    assert!(imp::walk(&dir, Path::new("up/secret"), OFlags::RDONLY).is_err());
    assert!(imp::walk(&dir, Path::new("out"), OFlags::RDONLY).is_err());
}

/// The reviewer's race: a folder swapped with a link out of the project
/// (RENAME_EXCHANGE) while reading through it. Checking a path and opening
/// it after returned the outside file 1084 times in 3000; opening beneath
/// the project's fd never does.
#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn a_folder_swapped_for_a_link_never_reads_outside() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let (tmp, root, data) = project();
    let base = std::fs::canonicalize(tmp.path()).unwrap();
    let outside = base.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("f"), "OUTSIDE").unwrap();
    std::fs::create_dir_all(root.join("d")).unwrap();
    std::fs::write(root.join("d/f"), "inside").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("l")).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let swapper = {
        let (stop, root) = (stop.clone(), root.clone());
        std::thread::spawn(move || {
            let mut swaps = 0u64;
            while !stop.load(Ordering::Relaxed) {
                rustix::fs::renameat_with(
                    rustix::fs::CWD,
                    root.join("d"),
                    rustix::fs::CWD,
                    root.join("l"),
                    rustix::fs::RenameFlags::EXCHANGE,
                )
                .unwrap();
                swaps += 1;
            }
            swaps
        })
    };
    // The old way, check then read, for comparison: it reads outside while
    // the swapper runs (printed, not asserted: a race).
    let mut naive_outside = 0;
    for _ in 0..3000 {
        let Ok(real) = std::fs::canonicalize(root.join("d/f")) else { continue };
        if real.starts_with(&root) && std::fs::read_to_string(root.join("d/f")).is_ok_and(|t| t == "OUTSIDE") {
            naive_outside += 1;
        }
    }
    let (mut inside, mut outside_reads, mut refused) = (0, 0, 0);
    for _ in 0..3000 {
        match read(&root, "d/f", &data) {
            Ok(read) if read.text.as_deref() == Some("OUTSIDE") => outside_reads += 1,
            Ok(_) => inside += 1,
            Err(_) => refused += 1,
        }
    }
    stop.store(true, Ordering::Relaxed);
    let swaps = swapper.join().unwrap();
    eprintln!(
        "swap race: check-then-read {naive_outside} of 3000 outside; peer_files {inside} inside, {refused} refused, {outside_reads} outside; {swaps} swaps"
    );
    assert!(swaps > 100, "the swapper ran ({swaps} swaps)");
    assert_eq!(outside_reads, 0, "{outside_reads} of 3000 reads returned the file outside the project");
}

#[test]
fn a_folder_with_zend_data_or_the_home_folder_is_never_shared() {
    let tmp = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(tmp.path()).unwrap();
    let home = base.join("home");
    let data = home.join(".zend");
    for dir in [".ssh", ".config/zend", ".aws", ".gnupg", ".pi", ".claude", ".codex", ".zend/repos", "Projects/app"] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    let elsewhere = base.join("var-zend");
    std::fs::create_dir_all(elsewhere.join("inside")).unwrap();
    for (root, why) in [
        (data.clone(), "data"),
        (data.join("repos"), "data"),
        (home.clone(), "home"),
        (base.clone(), "home"),
        (PathBuf::from("/"), "home"),
        (home.join(".ssh"), "hidden"),
        (home.join(".config/zend"), "hidden"),
        (home.join(".aws"), "hidden"),
        (home.join(".gnupg"), "hidden"),
        (home.join(".pi"), "hidden"),
        (home.join(".claude"), "hidden"),
        (home.join(".codex"), "hidden"),
    ] {
        let refused = unshareable(&root, &data, Some(&home)).unwrap_or_default();
        let expected = match why {
            "data" => "data folder",
            "home" => "home folder",
            _ => "hidden folder",
        };
        assert!(refused.contains(expected) || refused.contains("data folder"), "{}: {refused:?}", root.display());
    }
    // A data folder outside home: the folder holding it, and folders in it.
    assert!(unshareable(&base, &elsewhere, None).is_some());
    assert!(unshareable(&elsewhere.join("inside"), &elsewhere, None).is_some());
    // A project is fine.
    assert_eq!(unshareable(&home.join("Projects/app"), &data, Some(&home)), None);
}

#[test]
fn files_holding_secrets_by_name_are_never_shared() {
    let (_tmp, root, data) = project();
    let secrets = [
        ".env", ".env.local", ".ENV.production", ".netrc", ".npmrc", ".pypirc",
        "cert.pem", "src/tls.key", "id_ed25519", "id_rsa.pub", ".git/config",
    ];
    std::fs::create_dir_all(root.join(".git")).unwrap();
    for name in secrets {
        std::fs::write(root.join(name), "TOKEN").unwrap();
        assert!(denied(Path::new(name)), "{name}");
        let refused = read(&root, name, &data).unwrap_err();
        assert!(refused.contains("secrets"), "{name}: {refused}");
    }
    for name in ["readme.md", "src/a.txt", "environment.md", "config", "keys.txt", ".git/HEAD"] {
        assert!(!denied(Path::new(name)), "{name}");
    }
    let names = |dir: &str| list(&root, dir, &data).unwrap().into_iter().map(|e| e.name).collect::<Vec<_>>();
    let top = names("");
    assert!(top.contains(&"src".to_string()) && top.contains(&".git".to_string()), "{top:?}");
    for hidden in [".env", ".env.local", ".netrc", ".npmrc", ".pypirc", "cert.pem", "id_ed25519"] {
        assert!(!top.contains(&hidden.to_string()), "{hidden} listed: {top:?}");
    }
    assert!(!names(".git").contains(&"config".to_string()));
    assert!(!names("src").contains(&"tls.key".to_string()));
}

/// A FIFO in a project doesn't hang the read.
#[cfg(target_os = "linux")]
#[test]
fn a_fifo_is_refused_without_waiting() {
    let (_tmp, root, data) = project();
    rustix::fs::mknodat(rustix::fs::CWD, root.join("pipe"), rustix::fs::FileType::Fifo, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR, 0).unwrap();
    let refused = read(&root, "pipe", &data).unwrap_err();
    assert!(refused.contains("not a file"), "{refused}");
}
