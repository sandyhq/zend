//! What a Node CLI's launcher really runs, for the sandbox
//! ([`crate::sandbox::Launch`]): the node interpreter and the script, and the
//! folders that script loads its packages from.
//!
//! An agent CLI installed by a Node package manager is a launcher in a bin
//! folder: pnpm's shim (a `/bin/sh` script that execs `$basedir/node` on
//! `../global/v11/<hash>/node_modules/<package>/<bin>.js`, both links into
//! pnpm's store), or npm's link to the package's `#!/usr/bin/env node`
//! script. Binding the launcher's folder shows none of what it loads. So the
//! sandbox runs node on the script itself, and binds read-only only node's
//! real file and the script's package with its dependencies, each at its
//! real path, with pnpm's dependency links recreated.
//!
//! A pnpm shim is read, not run: only the form pnpm writes is accepted (its
//! `# cmd-shim-target=` line, `exec` lines that run node on that one target
//! and nothing else). Anything else (another target, options for node,
//! `NODE_PATH`/`NODE_OPTIONS`, `eval`) is refused with the reason, never
//! answered by binding more.
use std::collections::{BTreeSet, VecDeque};
use std::path::{Component, Path, PathBuf};

/// The most packages one script may load: a guard against a dependency
/// graph that never ends, far above an agent CLI's few hundred.
const MAX_PACKAGES: usize = 5000;

/// How the sandbox starts a launcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// Not a Node launcher (a native binary, another script): run as it is.
    Other,
    /// Run `node` on `script` (both real paths) in place of the launcher.
    Node { node: PathBuf, script: PathBuf },
}

/// What `exe` runs: [`Entry::Node`] for a pnpm shim or a node script,
/// [`Entry::Other`] for anything else, or why a Node launcher is refused.
/// `node_on_path` finds `node` the way the launcher's `command -v node`
/// (or `env node`) would.
pub fn resolve(exe: &Path, node_on_path: &dyn Fn() -> Option<PathBuf>) -> Result<Entry, String> {
    let real = std::fs::canonicalize(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let Some(text) = script_text(&real) else { return Ok(Entry::Other) };
    let first = text.lines().next().unwrap_or_default();
    if text.contains("cmd-shim-target=") {
        return shim(&real, &text, node_on_path);
    }
    let mut words = first.trim_start_matches("#!").split_whitespace();
    let interpreter = words.next().unwrap_or_default();
    let rest: Vec<&str> = words.collect();
    let named = |path: &str, name: &str| Path::new(path).file_name().is_some_and(|n| n == name);
    let node = if named(interpreter, "env") && rest.first() == Some(&"node") {
        if rest.len() > 1 {
            return Err(format!("{} starts node with options ({first}): not run sandboxed", real.display()));
        }
        node_on_path().ok_or_else(|| format!("{} runs node, and there is no node on the PATH", real.display()))?
    } else if named(interpreter, "node") {
        if !rest.is_empty() {
            return Err(format!("{} starts node with options ({first}): not run sandboxed", real.display()));
        }
        PathBuf::from(interpreter)
    } else if named(interpreter, "env") && rest.iter().any(|w| w.contains("node")) {
        return Err(format!("{} starts node in a way Zend doesn't read ({first}): not run sandboxed", real.display()));
    } else {
        return Ok(Entry::Other);
    };
    let node = std::fs::canonicalize(&node).map_err(|e| format!("its node {}: {e}", node.display()))?;
    Ok(Entry::Node { node, script: real })
}

/// The text of `path` when it is a script (`#!`), at most 64 KiB of it.
fn script_text(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(64 * 1024).read_to_end(&mut bytes).ok()?;
    bytes.starts_with(b"#!").then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// A pnpm shim at `real` (its real path): node and the one target it runs.
fn shim(real: &Path, text: &str, node_on_path: &dyn Fn() -> Option<PathBuf>) -> Result<Entry, String> {
    let refuse = |why: String| Err(format!("the pnpm shim {} {why}: not run sandboxed", real.display()));
    if text.lines().next().map(str::trim_end) != Some("#!/bin/sh") {
        return refuse("does not start with #!/bin/sh".into());
    }
    let targets: Vec<&str> = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix("# cmd-shim-target="))
        .collect();
    let [target] = targets.as_slice() else {
        return refuse(format!("names {} targets, not one", targets.len()));
    };
    let target = Path::new(target.trim());
    if !target.is_absolute() {
        return refuse(format!("names a relative target {}", target.display()));
    }
    let target = std::fs::canonicalize(target).map_err(|e| format!("the pnpm shim {}'s target {}: {e}", real.display(), target.display()))?;
    let dir = real.parent().unwrap_or(Path::new("/"));
    let mut runs = 0;
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') {
            continue;
        }
        if ["NODE_PATH", "NODE_OPTIONS", "export ", "eval "].iter().any(|word| line.contains(word)) {
            return refuse(format!("sets up more than node and its target ({line})"));
        }
        if !(line.starts_with("exec ") || line.contains(" exec ")) {
            continue;
        }
        let words = words(line).ok_or_else(|| format!("the pnpm shim {} has a line Zend doesn't read ({line}): not run sandboxed", real.display()))?;
        let program_ok = |w: &str| ["\"$basedir/node\"", "\"$basedir/node.exe\"", "node", "node.exe"].contains(&w);
        let script = match &words[..] {
            [exec, program, script, all] if *exec == "exec" && program_ok(program) && *all == "\"$@\"" => script,
            _ => return refuse(format!("runs something other than node on its target ({line})")),
        };
        let rel = script
            .strip_prefix("\"$basedir/")
            .or_else(|| script.strip_prefix("\"$basedir_win/"))
            .and_then(|s| s.strip_suffix('"'))
            .filter(|rel| !rel.contains(['$', '`', '\\', '"']));
        let Some(rel) = rel else {
            return refuse(format!("runs a script Zend doesn't read ({line})"));
        };
        if std::fs::canonicalize(dir.join(rel)).ok().as_ref() != Some(&target) {
            return refuse(format!("runs {rel}, not its target {}", target.display()));
        }
        runs += 1;
    }
    if runs == 0 {
        return refuse("runs nothing".into());
    }
    if !target.is_file() {
        return refuse(format!("names {}, which is not a file", target.display()));
    }
    // What the shim does on Linux: its own `node` beside it, else the PATH's.
    let beside = dir.join("node");
    let node = if runnable(&beside) {
        beside
    } else {
        node_on_path().ok_or_else(|| format!("the pnpm shim {} has no node beside it or on the PATH", real.display()))?
    };
    let node = std::fs::canonicalize(&node).map_err(|e| format!("its node {}: {e}", node.display()))?;
    Ok(Entry::Node { node, script: target })
}

/// `line` split into shell words, quotes kept; `None` for anything beyond
/// plain words and double-quoted ones (escapes, single quotes, `$(`).
fn words(line: &str) -> Option<Vec<String>> {
    if line.contains(['\'', '\\', '`']) || line.contains("$(") || line.contains(';') {
        return None;
    }
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                word.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if quoted {
        return None;
    }
    if !word.is_empty() {
        out.push(word);
    }
    Some(out)
}

#[cfg(unix)]
fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(path: &Path) -> bool {
    path.is_file()
}

/// The first `node` on `PATH` (as `command -v node` finds it), else on the
/// login shell's.
pub fn node_on_path() -> Option<PathBuf> {
    [std::env::var_os("PATH"), crate::shell_env::user_path()]
        .into_iter()
        .flatten()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join("node"))
        .find(|node| runnable(node))
}

/// What `script` loads, to bind read-only: its package's folder and every
/// dependency's (`dependencies`, `optionalDependencies`, `peerDependencies`
/// that are installed), each its real path; and the links Node finds them
/// through (pnpm's `<hash>/node_modules/<dep>`), as (target, link).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Closure {
    pub dirs: Vec<PathBuf>,
    pub links: Vec<(PathBuf, PathBuf)>,
}

pub fn closure(script: &Path) -> Result<Closure, String> {
    let script = std::fs::canonicalize(script).map_err(|e| format!("{}: {e}", script.display()))?;
    let start = script.parent().unwrap_or(Path::new("/")).to_path_buf();
    // Its package: the nearest folder up with a package.json, not past a
    // node_modules (a lone script outside any package: its folder).
    let root = start
        .ancestors()
        .take_while(|dir| dir.file_name().is_some_and(|n| n != "node_modules"))
        .find(|dir| dir.join("package.json").is_file())
        .unwrap_or(start.as_path())
        .to_path_buf();
    let mut out = Closure::default();
    let mut seen = BTreeSet::from([root.clone()]);
    let mut linked = BTreeSet::new();
    let mut queue = VecDeque::from([root.clone()]);
    out.dirs.push(root);
    while let Some(dir) = queue.pop_front() {
        for name in dependencies(&dir) {
            let Some(found) = lookup(&dir, &name) else { continue };
            let real = std::fs::canonicalize(&found).map_err(|e| format!("{}: {e}", found.display()))?;
            let parent = found.parent().unwrap_or(Path::new("/"));
            if std::fs::canonicalize(parent).ok().as_deref() != Some(parent) {
                return Err(format!("{} lies behind a link Zend doesn't follow", found.display()));
            }
            if std::fs::symlink_metadata(&found).is_ok_and(|m| m.file_type().is_symlink()) && linked.insert(found.clone()) {
                let target = std::fs::read_link(&found).map_err(|e| format!("{}: {e}", found.display()))?;
                out.links.push((target, found));
            }
            if seen.insert(real.clone()) {
                if seen.len() > MAX_PACKAGES {
                    return Err(format!("{} loads more than {MAX_PACKAGES} packages", script.display()));
                }
                out.dirs.push(real.clone());
                queue.push_back(real);
            }
        }
    }
    Ok(out)
}

/// The packages `dir`'s package.json depends on, by name (valid names only).
fn dependencies(dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(dir.join("package.json")) else { return Vec::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    ["dependencies", "optionalDependencies", "peerDependencies"]
        .iter()
        .filter_map(|key| json.get(key)?.as_object())
        .flat_map(|deps| deps.keys().cloned())
        .filter(|name| package_name(name))
        .collect()
}

/// `name` or `@scope/name`, nothing that leaves node_modules.
fn package_name(name: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    let plain = |p: &str| !p.is_empty() && p != "." && p != ".." && !p.contains('\\');
    match parts[..] {
        [one] => plain(one) && !one.starts_with('@'),
        [scope, one] => scope.starts_with('@') && plain(&scope[1..]) && plain(one),
        _ => false,
    }
}

/// Where Node finds package `name` from a module in `dir`: the first
/// `node_modules/<name>` up from it (no `node_modules/node_modules`).
fn lookup(dir: &Path, name: &str) -> Option<PathBuf> {
    dir.ancestors()
        .filter(|d| d.file_name().is_none_or(|n| n != "node_modules"))
        .map(|d| d.join("node_modules").join(name))
        .find(|candidate| std::fs::symlink_metadata(candidate).is_ok())
        .filter(|found| !found.components().any(|c| c == Component::ParentDir))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The shim pnpm 11 wrote for pi on a real device, with its target.
    fn pnpm_shim(target_rel: &str, target_abs: &Path) -> String {
        format!(
            r#"#!/bin/sh
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
  exec "$basedir/node.exe"  "$basedir_win/{t}" "$@"
elif [ -x "$basedir/node" ]; then
  exec "$basedir/node"  "$basedir/{t}" "$@"
elif command -v node >/dev/null 2>&1; then
  exec node  "$basedir/{t}" "$@"
elif [ -n "$exe" ] && command -v node.exe >/dev/null 2>&1; then
  exec node.exe  "$basedir_win/{t}" "$@"
else
  exec node  "$basedir/{t}" "$@"
fi
# cmd-shim-target={a}
"#,
            t = target_rel,
            a = target_abs.display()
        )
    }

    /// pnpm's layout: bin/pi, bin/node a link into the store, the global
    /// package a link into the store.
    fn pnpm_layout(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let node = root.join("store/v11/links/@/node/26.0.0/h0/node_modules/node/bin/node");
        write(&node, "#!/bin/sh\nexec /usr/bin/node \"$@\"\n");
        std::fs::create_dir_all(root.join("bin")).unwrap();
        symlink(&node, root.join("bin/node")).unwrap();
        let package = root.join("store/v11/links/@s/agent/1.0.0/ha/node_modules/@s/agent");
        write(&package.join("dist/cli.js"), "#!/usr/bin/env node\n");
        std::fs::create_dir_all(root.join("global/v11/g1/node_modules/@s")).unwrap();
        symlink("../../../../../store/v11/links/@s/agent/1.0.0/ha/node_modules/@s/agent", root.join("global/v11/g1/node_modules/@s/agent")).unwrap();
        let rel = "../global/v11/g1/node_modules/@s/agent/dist/cli.js";
        write(&root.join("bin/pi"), &pnpm_shim(rel, &root.join(rel.trim_start_matches("../"))));
        (root.join("bin/pi"), std::fs::canonicalize(&node).unwrap(), std::fs::canonicalize(package.join("dist/cli.js")).unwrap())
    }

    #[test]
    fn a_pnpm_shim_resolves_to_its_node_and_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let (shim, node, script) = pnpm_layout(dir.path());
        assert_eq!(resolve(&shim, &|| None).unwrap(), Entry::Node { node, script });
    }

    #[test]
    fn a_pnpm_shim_without_node_beside_it_uses_the_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (shim, _, script) = pnpm_layout(dir.path());
        std::fs::remove_file(dir.path().join("bin/node")).unwrap();
        let elsewhere = dir.path().join("other/node");
        write(&elsewhere, "#!/bin/sh\n");
        let found = resolve(&shim, &|| Some(elsewhere.clone())).unwrap();
        assert_eq!(found, Entry::Node { node: std::fs::canonicalize(&elsewhere).unwrap(), script });
        assert!(resolve(&shim, &|| None).unwrap_err().contains("no node beside it or on the PATH"));
    }

    #[test]
    fn a_shim_that_does_more_than_pnpms_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (shim, _, _) = pnpm_layout(dir.path());
        let good = std::fs::read_to_string(&shim).unwrap();
        let rel = "../global/v11/g1/node_modules/@s/agent/dist/cli.js";
        let other = dir.path().join("evil.js");
        write(&other, "x");
        for (what, text) in [
            ("node options", good.replacen("exec \"$basedir/node\"  ", "exec \"$basedir/node\" --require /x.js ", 1)),
            ("NODE_PATH", good.replacen("exe=\"\"\n", "exe=\"\"\nexport NODE_PATH=\"/home\"\n", 1)),
            ("another script", good.replacen(&format!("\"$basedir/{rel}\""), "\"$basedir/../evil.js\"", 1)),
            ("two targets", format!("{good}# cmd-shim-target={}\n", other.display())),
            ("another program", good.replacen("exec node  ", "exec /bin/sh  ", 1)),
            ("no shebang", good.replacen("#!/bin/sh", "#!/bin/bash", 1)),
        ] {
            std::fs::write(&shim, &text).unwrap();
            let error = resolve(&shim, &|| Some(PathBuf::from("/usr/bin/node"))).unwrap_err();
            println!("{what}: {error}");
            assert!(error.contains("not run sandboxed"), "{what}: {error}");
        }
    }

    #[test]
    fn a_node_script_runs_on_the_node_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let node = dir.path().join("n/node");
        write(&node, "#!/bin/sh\n");
        let cli = dir.path().join("lib/node_modules/a/cli.js");
        write(&cli, "#!/usr/bin/env node\nconsole.log(1)\n");
        std::fs::create_dir_all(dir.path().join("bin")).unwrap();
        symlink("../lib/node_modules/a/cli.js", dir.path().join("bin/a")).unwrap();
        let script = std::fs::canonicalize(&cli).unwrap();
        let node = std::fs::canonicalize(&node).unwrap();
        assert_eq!(resolve(&dir.path().join("bin/a"), &|| Some(node.clone())).unwrap(), Entry::Node { node: node.clone(), script: script.clone() });
        write(&cli, &format!("#!{}\n", node.display()));
        assert_eq!(resolve(&cli, &|| None).unwrap(), Entry::Node { node, script });
        write(&cli, "#!/usr/bin/env -S node --flag\n");
        assert!(resolve(&cli, &|| None).unwrap_err().contains("not run sandboxed"));
        // Another script or a binary: as it is.
        write(&cli, "#!/bin/sh\necho hi\n");
        assert_eq!(resolve(&cli, &|| None).unwrap(), Entry::Other);
        assert_eq!(resolve(Path::new("/bin/true"), &|| None).unwrap(), Entry::Other);
    }

    #[test]
    fn the_closure_holds_the_package_its_dependencies_and_their_links_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let links = root.join("store/v11/links");
        let pkg = |name: &str, deps: &[&str]| {
            let at = links.join(format!("@/{name}/1.0.0/h-{name}/node_modules/{name}"));
            let deps: Vec<String> = deps.iter().map(|d| format!("\"{d}\":\"1\"")).collect();
            write(&at.join("package.json"), &format!("{{\"name\":\"{name}\",\"dependencies\":{{{}}}}}", deps.join(",")));
            write(&at.join("index.js"), "");
            at
        };
        let agent = pkg("agent", &["dep-a", "missing"]);
        let a = pkg("dep-a", &["dep-b", "agent"]);
        let b = pkg("dep-b", &[]);
        let _secret = pkg("secret", &[]);
        let link = |from: &Path, to: &Path, name: &str| {
            let path = from.parent().unwrap().join(name);
            symlink(to, &path).unwrap();
            path
        };
        let la = link(&agent, &a, "dep-a");
        let lb = link(&a, &b, "dep-b");
        let lc = link(&a, &agent, "agent");
        let mut found = closure(&agent.join("index.js")).unwrap();
        assert_eq!(found.dirs[0], agent);
        found.dirs.sort();
        found.links.sort();
        let mut dirs = vec![agent.clone(), a.clone(), b.clone()];
        dirs.sort();
        let mut links = vec![(a.clone(), la), (b.clone(), lb), (agent.clone(), lc)];
        links.sort();
        assert_eq!(found.dirs, dirs);
        assert_eq!(found.links, links);
        // npm's layout: dependencies inside the package, nothing more.
        let npm = root.join("lib/node_modules/@s/cli");
        write(&npm.join("package.json"), "{\"dependencies\":{\"x\":\"1\"}}");
        write(&npm.join("dist/cli.js"), "");
        write(&npm.join("node_modules/x/package.json"), "{}");
        write(&root.join("lib/node_modules/other/package.json"), "{}");
        let found = closure(&npm.join("dist/cli.js")).unwrap();
        assert_eq!(found.dirs, vec![npm.clone(), npm.join("node_modules/x")]);
        assert!(found.links.is_empty());
    }

    #[test]
    fn package_names_stay_in_node_modules() {
        for name in ["a", "@s/a", "a.b"] {
            assert!(package_name(name), "{name}");
        }
        for name in ["..", "../x", "@s/..", "a/b", "@s", "@s/a/b", ""] {
            assert!(!package_name(name), "{name}");
        }
    }
}
