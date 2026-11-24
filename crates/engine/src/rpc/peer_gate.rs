//! What a paired server (`zend_rpc::Caller::PeerServer`, over the peer link)
//! reaches of this engine, call by call. The transport let through only
//! the methods its grants open (`zend_rpc::peer_server_allowed`); here each
//! call's chat or project is held to those grants:
//!
//! - `Mutate`: `createChat` only, in a project of this device it may run in
//!   (`sessions.run:<p>` or `sessions.run.unconfined:<p>`), at the project's
//!   folder: no cwd, host, parent, branch or Cloud of its own. The chat is
//!   kept as that server's (`crate::peer_chats`) and, without `.unconfined`,
//!   marked `sandbox: "project"` before it exists: it runs confined or not
//!   at all (`SessionsEngine::sandbox_chat`).
//! - `QueueCommand`: its own chats (`sessions.run`), or any chat of a project
//!   it may run unconfined. A run is built here from the chat's row with
//!   only the sender's prompt (no cwd, MCP servers, env or files of its own).
//! - `WatchDocMessages`: as `QueueCommand`, or any chat here with `sessions.read`.
//! - `PeerListProjectFiles`, `PeerReadProjectFile`: a `projects.share:<p>`
//!   project's files, by a path from its root (never absolute, never `..`),
//!   checked again after symlinks are followed.
//!
//! Nothing is forwarded to another device for it (`targetDeviceId`).

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};
use zend_rpc::{Caller, Grants, RpcError, RpcReply, methods};

use super::{EngineRpc, MutateParams, QueueCommandParams};
use crate::peer_chats::valid_chat_id;
use zend_doc::SessionCommandPayload;

/// The most of a file `PeerReadProjectFile` returns as text.
pub(crate) const MAX_READ_BYTES: u64 = 1 << 20;
/// The most entries one listing returns.
const MAX_ENTRIES: usize = 5000;

pub(super) enum PeerGate {
    /// Served as any call is, with these params.
    Pass(Value),
    /// Answered here.
    Reply(RpcReply),
}

fn refused(why: impl Into<String>) -> RpcError {
    RpcError::Failed(why.into())
}

impl EngineRpc {
    /// `params` as a paired server may send them for `method`, an answer, or why
    /// not. Every other caller's pass untouched.
    pub(super) async fn peer_gate(&self, method: &str, params: Value) -> Result<PeerGate, RpcError> {
        let Caller::PeerServer { peer, grants } = zend_rpc::caller() else {
            return Ok(PeerGate::Pass(params));
        };
        // The transport checked it; checked again where it's served.
        zend_rpc::peer_server_allowed(method, &grants).map_err(refused)?;
        if params
            .get("targetDeviceId")
            .and_then(Value::as_str)
            .is_some_and(|target| target != self.doc_host.device_id())
        {
            return Err(refused(format!("{method}: this device answers for itself only")));
        }
        match method {
            methods::ENGINE_INFO
            | methods::LIST_HARNESSES
            | methods::LIST_MODELS
            | methods::WATCH_SESSIONS
            | methods::WATCH_CHATS => Ok(PeerGate::Pass(params)),
            methods::WATCH_DOC_MESSAGES => {
                let chat_id = params.get("chatId").and_then(Value::as_str).unwrap_or_default();
                self.peer_reaches(&peer, &grants, chat_id, true)?;
                Ok(PeerGate::Pass(params))
            }
            methods::QUEUE_COMMAND => self.peer_queue_command(&peer, &grants, params).map(PeerGate::Pass),
            methods::MUTATE => self.peer_create_chat(&peer, &grants, params).await.map(PeerGate::Reply),
            methods::PEER_LIST_PROJECT_FILES => self.peer_list_files(&grants, params).map(PeerGate::Reply),
            methods::PEER_READ_PROJECT_FILE => self.peer_read_file(&grants, params).map(PeerGate::Reply),
            _ => Err(refused(format!("{method} isn't open to your server"))),
        }
    }

    /// The chat `chat_id`, hosted here, when the server holding `peer` may run
    /// it (or, `reading`, read it) under `grants`.
    fn peer_reaches(&self, peer: &str, grants: &Grants, chat_id: &str, reading: bool) -> Result<zend_proto::Chat, RpcError> {
        let not_yours = || refused(format!("chat {chat_id}: not one your server may reach here"));
        let chat = self
            .workspace
            .chat(chat_id)
            .map_err(|e| RpcError::Failed(e.to_string()))?
            .filter(|chat| chat.device_id == self.doc_host.device_id())
            .ok_or_else(not_yours)?;
        if reading && grants.reads() {
            return Ok(chat);
        }
        let runs = chat.space_id.as_deref().and_then(|space| grants.runs_in(space));
        let own = self.sessions.peer_chats().peer(chat_id).as_deref() == Some(peer);
        match runs {
            Some(true) => Ok(chat),
            Some(false) if own => Ok(chat),
            _ => Err(not_yours()),
        }
    }

    fn peer_queue_command(&self, peer: &str, grants: &Grants, params: Value) -> Result<Value, RpcError> {
        let p: QueueCommandParams = zend_rpc::parse_params(params)?;
        let chat = self.peer_reaches(peer, grants, &p.chat_id, false)?;
        if !p.transfers.is_empty() {
            return Err(refused("A server sends no files with a message yet"));
        }
        let runs = matches!(p.command, SessionCommandPayload::Run { .. } | SessionCommandPayload::Steer { .. });
        if runs && let Some(sandbox) = self.sessions.chat_sandbox(&chat.id) {
            return Err(refused(format!(
                "This session must run sandboxed (sandbox: {}) and was not started: this build of Zend can't run sessions sandboxed yet",
                sandbox.as_str()
            )));
        }
        let command = match p.command {
            SessionCommandPayload::Run { request, message_id } => {
                // The chat's own: its folder, harness and model as its row
                // says, the sender's words only.
                let request = self
                    .doc_host
                    .request_from_chat_row(&chat.id, &request.prompt)
                    .ok_or_else(|| refused(format!("chat {}: no row to run", chat.id)))?;
                SessionCommandPayload::Run { request, message_id }
            }
            other => other,
        };
        Ok(json!({ "chatId": chat.id, "command": command }))
    }

    async fn peer_create_chat(&self, peer: &str, grants: &Grants, params: Value) -> Result<RpcReply, RpcError> {
        if params.get("op").and_then(Value::as_str) != Some("createChat") {
            return Err(refused("Mutate: a server may only create a chat (op createChat)"));
        }
        let p: MutateParams = zend_rpc::parse_params(params)?;
        let MutateParams::CreateChat {
            chat_id,
            space_id,
            device_id,
            config,
            branch,
            cwd,
            parent_chat_id,
            child_kind,
            notify_parent: _,
            cloud,
        } = p
        else {
            return Err(refused("Mutate: a server may only create a chat (op createChat)"));
        };
        let own_device = self.doc_host.device_id().to_string();
        let Some(space_id) = space_id else {
            return Err(refused("createChat: a server creates chats in a project (spaceId)"));
        };
        let Some(unconfined) = grants.runs_in(&space_id) else {
            return Err(refused(format!("createChat: your server may not run sessions in project {space_id}")));
        };
        if device_id.as_deref().is_some_and(|d| d != own_device)
            || cwd.is_some()
            || parent_chat_id.is_some()
            || child_kind.is_some()
            || cloud
            || branch.as_deref().is_some_and(|b| !b.is_empty())
        {
            return Err(refused("createChat: a server's chat runs at its project's folder (no deviceId, cwd, parent, branch or cloud)"));
        }
        let space = self
            .workspace
            .space(&space_id)
            .map_err(|e| RpcError::Failed(e.to_string()))?
            .filter(|space| space.device_id == own_device)
            .ok_or_else(|| refused(format!("createChat: no project {space_id} on this device")))?;
        if !valid_chat_id(&chat_id) {
            return Err(refused("createChat: chatId is letters, digits, - and _"));
        }
        if self.workspace.chat(&chat_id).map_err(|e| RpcError::Failed(e.to_string()))?.is_some() {
            return Err(refused(format!("createChat: chat {chat_id} exists")));
        }
        // Kept as the server's, and confined, before it exists: it never
        // runs unmarked.
        let chats = self.sessions.peer_chats();
        chats.note(&chat_id, peer).map_err(|e| RpcError::Failed(format!("createChat: {e}")))?;
        if !unconfined
            && let Err(error) = self.sessions.sandbox_chat(&chat_id, zend_proto::ChatSandbox::Project).await
        {
            chats.forget(&chat_id);
            return Err(RpcError::Failed(format!("createChat: couldn't mark it sandboxed: {error}")));
        }
        let created = self.mutate(MutateParams::CreateChat {
            chat_id: chat_id.clone(),
            space_id: Some(space.id.clone()),
            device_id: None,
            config,
            branch: None,
            cwd: None,
            parent_chat_id: None,
            child_kind: None,
            notify_parent: None,
            cloud: false,
        });
        if let Err(error) = created {
            chats.forget(&chat_id);
            return Err(error);
        }
        tracing::info!(chat = %chat_id, space = %space.id, unconfined, "peer link: a paired server created a chat");
        RpcReply::value(&json!({
            "ok": true,
            "chatId": chat_id,
            "deviceId": own_device,
            "sandbox": (!unconfined).then_some(zend_proto::ChatSandbox::Project),
        }))
    }

    /// The folder of `project` (a space on this device), canonical, when
    /// `grants` share it.
    fn peer_project_root(&self, grants: &Grants, project: &str) -> Result<PathBuf, RpcError> {
        if !grants.shares(project) {
            return Err(refused(format!("project {project} isn't shared with your server")));
        }
        let space = self
            .workspace
            .space(project)
            .map_err(|e| RpcError::Failed(e.to_string()))?
            .filter(|space| space.device_id == self.doc_host.device_id())
            .ok_or_else(|| refused(format!("no project {project} on this device")))?;
        std::fs::canonicalize(&space.path).map_err(|e| refused(format!("project {project}: its folder: {e}")))
    }

    fn peer_list_files(&self, grants: &Grants, params: Value) -> Result<RpcReply, RpcError> {
        let (project, path) = project_path(&params)?;
        let root = self.peer_project_root(grants, &project)?;
        let dir = inside(&root, &path)?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(|e| refused(format!("{path}: {e}")))?.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            let kind = if meta.is_symlink() { "link" } else if meta.is_dir() { "dir" } else { "file" };
            entries.push(json!({
                "name": entry.file_name().to_string_lossy(),
                "kind": kind,
                "size": meta.is_file().then_some(meta.len()),
            }));
        }
        entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        let truncated = entries.len() > MAX_ENTRIES;
        entries.truncate(MAX_ENTRIES);
        RpcReply::value(&json!({ "project": project, "path": path, "entries": entries, "truncated": truncated }))
    }

    fn peer_read_file(&self, grants: &Grants, params: Value) -> Result<RpcReply, RpcError> {
        let (project, path) = project_path(&params)?;
        let root = self.peer_project_root(grants, &project)?;
        let file = inside(&root, &path)?;
        let meta = std::fs::metadata(&file).map_err(|e| refused(format!("{path}: {e}")))?;
        if !meta.is_file() {
            return Err(refused(format!("{path}: not a file")));
        }
        let text = if meta.len() <= MAX_READ_BYTES {
            std::fs::read(&file).ok().and_then(|bytes| String::from_utf8(bytes).ok())
        } else {
            None
        };
        RpcReply::value(&json!({ "project": project, "path": path, "size": meta.len(), "text": text }))
    }
}

/// `{project, path?}` of a file call.
fn project_path(params: &Value) -> Result<(String, String), RpcError> {
    let project = params.get("project").and_then(Value::as_str).filter(|p| !p.is_empty());
    let Some(project) = project else {
        return Err(RpcError::BadParams("project (its id) is required".into()));
    };
    let path = params.get("path").and_then(Value::as_str).unwrap_or_default();
    Ok((project.to_string(), path.to_string()))
}

/// `relative` under `root` (canonical), when it names a path from the root and
/// stays inside it once every symlink is followed.
pub(crate) fn inside(root: &Path, relative: &str) -> Result<PathBuf, RpcError> {
    let rel = Path::new(relative);
    if rel.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(refused(format!(
            "{relative}: a path from the project's root (no leading /, no ..)"
        )));
    }
    let resolved = std::fs::canonicalize(root.join(rel)).map_err(|e| refused(format!("{relative}: {e}")))?;
    if !resolved.starts_with(root) {
        return Err(refused(format!("{relative}: outside the project (a link out of it)")));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_stays_inside_its_project() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        let outside = tmp.path().join("secret");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.txt"), "a").unwrap();
        std::fs::write(&outside, "s").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("out")).unwrap();
            std::os::unix::fs::symlink(tmp.path(), root.join("up")).unwrap();
            std::os::unix::fs::symlink("src/a.txt", root.join("in")).unwrap();
        }
        let root = std::fs::canonicalize(&root).unwrap();
        assert!(inside(&root, "src/a.txt").is_ok());
        assert!(inside(&root, "").is_ok());
        assert!(inside(&root, "./src").is_ok());
        for bad in ["../secret", "src/../../secret", "/etc/passwd", outside.to_str().unwrap()] {
            let refused = inside(&root, bad).unwrap_err().to_string();
            assert!(refused.contains("from the project's root"), "{bad}: {refused}");
        }
        #[cfg(unix)]
        {
            assert!(inside(&root, "in").is_ok());
            for link in ["out", "up/secret", "up"] {
                let refused = inside(&root, link).unwrap_err().to_string();
                assert!(refused.contains("outside the project"), "{link}: {refused}");
            }
        }
    }
}
