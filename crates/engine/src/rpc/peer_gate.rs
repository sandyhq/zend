//! What a paired server (`zend_rpc::Caller::PeerServer`, over the peer link)
//! reaches of this engine, call by call. The transport let through only
//! the methods its grants open (`zend_rpc::peer_server_allowed`); here each
//! call's chat or project is held to those grants:
//!
//! - `Mutate`: `createChat` only, in a project of this device it may run in
//!   (`sessions.run:<p>` or `sessions.run.unconfined:<p>`), at the project's
//!   folder: no cwd, host, parent, branch or Cloud of its own. The chat is
//!   kept as that server's (`crate::peer_chats`) and, without `.unconfined`,
//!   sandboxed to the project's registered folder before it exists
//!   (`SessionsEngine::sandbox_chat`): its agent runs under bubblewrap in
//!   that folder, or not at all.
//!   A confined chat's sandbox, access and plan mode are this side's
//!   (workspace-write, supervised, no plan), whatever the server sends; its
//!   model options must be ones its model lists.
//! - `QueueCommand`: its own chats (`sessions.run`), or any chat of a project
//!   it may run unconfined. A run is built here from the chat's row with
//!   only the sender's prompt (no cwd, MCP servers, env or files of its own).
//!   A chat of its own started unconfined doesn't run once the grant for its
//!   project is only `sessions.run` (no sandbox mark: refused).
//! - `WatchDocMessages`: as `QueueCommand`, or any chat here with `sessions.read`.
//! - `WatchChats`, `WatchSessions` (`sessions.read`): this device's own
//!   chats and sessions only; a chat's folder as a path from its project's
//!   root, never an absolute one.
//! - `PeerListProjectFiles`, `PeerReadProjectFile`: a `projects.share:<p>`
//!   project's files, by a path from its root, opened beneath the project's
//!   folder ([`crate::peer_files`]): no link followed, no file with other
//!   names, no secret by name, never a project at or around Zend's data or
//!   the home folder.
//!
//! An agent running in a chat a server started (or in a chat that chat
//! made) calls as that server, with its grants as they are now
//! ([`EngineRpc::peer_agent`]): its token is no way past them. Its own
//! children are kept as that server's too and, if it is confined, confined
//! to its folder, in its project only.
//!
//! Nothing is forwarded to another device for it (`targetDeviceId`).

use std::path::PathBuf;

use futures::StreamExt as _;

use serde_json::{Value, json};
use zend_rpc::{Caller, Grants, RpcError, RpcReply, methods};

use super::{EngineRpc, MutateParams, QueueCommandParams};
use crate::peer_chats::valid_chat_id;
use zend_doc::SessionCommandPayload;
use zend_proto::{AccessMode, ChatConfig, SandboxLevel};

tokio::task_local! {
    /// The chat whose agent calls, as its server ([`EngineRpc::peer_agent`]).
    static PEER_AGENT_CHAT: String;
}

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
    /// The server an agent's call speaks for: an agent (its MCP token) in a
    /// chat a server started, or one that chat made, calls as that server,
    /// with the grants this side gives it now (none once unpaired). What it
    /// waits on stays its own chat's (`AwaitBackground`, the agent gate).
    pub(super) fn peer_agent(&self, method: &str) -> Option<(Caller, String)> {
        let Caller::Agent { chat: Some(chat) } = zend_rpc::caller() else {
            return None;
        };
        if method == methods::AWAIT_BACKGROUND {
            return None;
        }
        let peer = self.sessions.peer_chats().peer(&chat)?;
        let grants = crate::peer_link::kept_grants(self.repos.data_dir(), &peer).unwrap_or_default();
        Some((Caller::PeerServer { peer, grants }, chat))
    }

    /// `handle` as the server `peer_agent` named.
    pub(super) async fn as_peer_agent(
        &self,
        (caller, chat): (Caller, String),
        method: &str,
        params: Value,
    ) -> Result<RpcReply, RpcError> {
        tracing::debug!(%chat, method, "peer link: an agent in a server's chat calls as that server");
        PEER_AGENT_CHAT
            .scope(chat, zend_rpc::with_caller(caller, zend_rpc::RpcService::handle(self, method, params)))
            .await
    }

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
            | methods::LIST_MODELS => Ok(PeerGate::Pass(params)),
            methods::WATCH_CHATS => Ok(PeerGate::Reply(RpcReply::Stream(self.peer_watch_chats()))),
            methods::WATCH_SESSIONS => Ok(PeerGate::Reply(RpcReply::Stream(self.peer_watch_sessions()))),
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
        let marked = self.sessions.chat_sandbox(&chat.id);
        // Started unconfined, now granted only confined: not run unmarked.
        if runs
            && marked.is_none()
            && chat.space_id.as_deref().and_then(|space| grants.runs_in(space)) == Some(false)
        {
            return Err(refused(format!(
                "chat {}: started unconfined, and your server may now run sessions in its project only sandboxed: it doesn't run",
                chat.id
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
        // An agent in a server's chat makes children of its own chat only.
        let calling = PEER_AGENT_CHAT.try_with(Clone::clone).ok();
        let parent_ok = parent_chat_id.is_none() || parent_chat_id == calling;
        if device_id.as_deref().is_some_and(|d| d != own_device)
            || cwd.is_some()
            || !parent_ok
            || (child_kind.is_some() && parent_chat_id.is_none())
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
        // The folder it runs confined to: a confined chat's, its project's
        // registered folder (never one from the request); a chat a confined
        // chat makes, its parent's, in its parent's project only, whatever
        // the grant.
        let mut made_copy = None;
        let root = match calling.as_deref().and_then(|c| self.sessions.chat_sandbox(c)) {
            Some(mark) => {
                let calling = calling.as_deref().unwrap_or_default();
                let parent_space = self
                    .workspace
                    .chat(calling)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .and_then(|chat| chat.space_id);
                if parent_space.as_deref() != Some(space.id.as_str()) {
                    return Err(refused(format!(
                        "createChat: a chat that sandboxed chat {calling} starts stays in its project ({})",
                        parent_space.as_deref().unwrap_or("none")
                    )));
                }
                Some(mark.root.ok_or_else(|| refused(format!("createChat: chat {calling}'s sandbox has no project folder")))?)
            }
            // A chat of its own: a working copy of the project in Zend's
            // data, never the owner's folder (step 4d-2).
            None if !unconfined => {
                let copies = self
                    .sessions
                    .peer_copies()
                    .ok_or_else(|| refused("createChat: this engine keeps no working copies for servers' chats"))?;
                let copy = copies
                    .create(&chat_id, peer, &space.id, std::path::Path::new(&space.path))
                    .map_err(|e| refused(format!("createChat: project {}: its working copy: {e}", space.id)))?;
                made_copy = Some(copies);
                Some(copy)
            }
            None => None,
        };
        let undo_copy = |made: &Option<crate::peer_copies::Copies>| {
            if let Some(copies) = made
                && let Err(error) = copies.discard(&chat_id)
            {
                tracing::warn!(chat = %chat_id, %error, "peer link: a working copy of a chat not made stays");
            }
        };
        let confined = root.is_some();
        // None would run with the row's fallback (full access): a confined
        // chat gets this device's default harness under this side's pins.
        let config = match config {
            Some(config) if confined => self.peer_confined_config(config).await.map(Some),
            None if confined => self
                .peer_confined_config(ChatConfig {
                    harness: self.doc_host.harness_for(&chat_id),
                    model: None,
                    reasoning: None,
                    model_options: Default::default(),
                    sandbox: SandboxLevel::WorkspaceWrite,
                    access: AccessMode::Supervised,
                    plan: false,
                })
                .await
                .map(Some),
            other => Ok(other),
        };
        let config = match config {
            Ok(config) => config,
            Err(error) => {
                undo_copy(&made_copy);
                return Err(error);
            }
        };
        // Kept as the server's, and confined, before it exists: it never
        // runs unmarked.
        let chats = self.sessions.peer_chats();
        if let Err(e) = chats.note(&chat_id, peer) {
            undo_copy(&made_copy);
            return Err(RpcError::Failed(format!("createChat: {e}")));
        }
        if let Some(root) = &root
            && let Err(error) = self.sessions.sandbox_chat(&chat_id, root).await
        {
            // Marked all the same, with no folder: it never runs.
            chats.forget(&chat_id);
            undo_copy(&made_copy);
            return Err(refused(format!("createChat: it can't run sandboxed in project {}: {error}", space.id)));
        }
        let created = self.mutate(MutateParams::CreateChat {
            chat_id: chat_id.clone(),
            space_id: Some(space.id.clone()),
            device_id: None,
            config,
            branch: None,
            cwd: None,
            parent_chat_id,
            child_kind,
            notify_parent: None,
            cloud: false,
        });
        if let Err(error) = created {
            chats.forget(&chat_id);
            undo_copy(&made_copy);
            return Err(error);
        }
        tracing::info!(chat = %chat_id, space = %space.id, confined, "peer link: a paired server created a chat");
        RpcReply::value(&json!({
            "ok": true,
            "chatId": chat_id,
            "deviceId": own_device,
            "sandbox": confined.then_some(zend_proto::ChatSandbox::Project),
        }))
    }

    /// A confined chat's config: this side's sandbox, access and plan mode;
    /// the server's harness and model, with model options its model lists.
    async fn peer_confined_config(&self, mut config: ChatConfig) -> Result<ChatConfig, RpcError> {
        config.sandbox = SandboxLevel::WorkspaceWrite;
        config.access = AccessMode::Supervised;
        config.plan = false;
        if config.model_options.is_empty() {
            return Ok(config);
        }
        let bad = |why: String| refused(format!("createChat: modelOptions: {why}"));
        let listed = zend_rpc::RpcService::handle(self, methods::LIST_MODELS, json!({ "harness": config.harness }))
            .await
            .map_err(|e| bad(format!("couldn't list the harness's models to check them: {e}")))?;
        let RpcReply::Value(listed) = listed else {
            return Err(bad("couldn't list the harness's models to check them".into()));
        };
        let models: Vec<zend_proto::Model> = serde_json::from_value(listed.get("models").cloned().unwrap_or(listed))
            .map_err(|e| bad(format!("the harness's models: {e}")))?;
        let model = match config.model.as_deref() {
            Some(id) => models.iter().find(|m| m.id == id),
            None => models.iter().find(|m| m.default).or(models.first()),
        }
        .ok_or_else(|| bad("no such model on this device".into()))?;
        for (option, choice) in &config.model_options {
            let known = model.options.iter().find(|o| &o.id == option).is_some_and(|o| {
                choice.as_str().is_some_and(|choice| o.choices.iter().any(|c| c.id == choice))
            });
            if !known {
                return Err(bad(format!("{option}={choice} isn't one of the model's options")));
            }
        }
        Ok(config)
    }

    /// This device's own chats, each one's folder from its project's root.
    fn peer_watch_chats(&self) -> futures::stream::BoxStream<'static, Value> {
        let own = self.doc_host.device_id().to_string();
        let workspace = self.workspace.clone();
        super::watch_stream(self.workspace.watch_chats())
            .map(move |chats| {
                let spaces = workspace.read_spaces().unwrap_or_default();
                seen_chats(chats, &own, &spaces)
            })
            .boxed()
    }

    /// This device's own sessions.
    fn peer_watch_sessions(&self) -> futures::stream::BoxStream<'static, Value> {
        let own = self.doc_host.device_id().to_string();
        super::watch_stream(self.workspace.merged_sessions_watch(self.sessions.watch_sessions()))
            .map(move |sessions| only_own(sessions, &own))
            .boxed()
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
        let root = std::fs::canonicalize(&space.path).map_err(|e| refused(format!("project {project}: its folder: {e}")))?;
        if let Some(why) = crate::peer_files::unshareable(&root, self.repos.data_dir(), crate::peer_files::home().as_deref()) {
            return Err(refused(format!("project {project} isn't shared: {why}")));
        }
        Ok(root)
    }

    fn peer_list_files(&self, grants: &Grants, params: Value) -> Result<RpcReply, RpcError> {
        let (project, path) = project_path(&params)?;
        let root = self.peer_project_root(grants, &project)?;
        let mut entries: Vec<Value> = crate::peer_files::list(&root, &path, self.repos.data_dir())
            .map_err(refused)?
            .into_iter()
            .map(|e| json!({ "name": e.name, "kind": e.kind, "size": e.size }))
            .collect();
        let truncated = entries.len() > MAX_ENTRIES;
        entries.truncate(MAX_ENTRIES);
        RpcReply::value(&json!({ "project": project, "path": path, "entries": entries, "truncated": truncated }))
    }

    fn peer_read_file(&self, grants: &Grants, params: Value) -> Result<RpcReply, RpcError> {
        let (project, path) = project_path(&params)?;
        let root = self.peer_project_root(grants, &project)?;
        let read = crate::peer_files::read(&root, &path, self.repos.data_dir()).map_err(refused)?;
        RpcReply::value(&json!({ "project": project, "path": path, "size": read.size, "text": read.text }))
    }
}

/// Of a list of rows, those with `deviceId` `own`.
fn only_own(rows: Value, own: &str) -> Value {
    match rows {
        Value::Array(rows) => Value::Array(
            rows.into_iter().filter(|row| row.get("deviceId").and_then(Value::as_str) == Some(own)).collect(),
        ),
        _ => Value::Array(Vec::new()),
    }
}

/// Chats as a server sees them: this device's own, a chat's folder as a path
/// from its project's root (left out when it's elsewhere), no other absolute
/// path (resume folder, checkout, repository context).
pub(crate) fn seen_chats(chats: Value, own: &str, spaces: &[zend_proto::Space]) -> Value {
    let Value::Array(chats) = only_own(chats, own) else { unreachable!() };
    Value::Array(
        chats
            .into_iter()
            .map(|mut chat| {
                let Some(row) = chat.as_object_mut() else { return chat };
                for key in ["harnessSessionCwd", "harnessSessionId", "sourceContext", "checkoutId"] {
                    row.remove(key);
                }
                let space = row.get("spaceId").and_then(Value::as_str).and_then(|id| spaces.iter().find(|s| s.id == id));
                let relative = row.get("cwd").and_then(Value::as_str).and_then(|cwd| {
                    let rest = std::path::Path::new(cwd).strip_prefix(&space?.path).ok()?;
                    let rest = rest.to_string_lossy();
                    Some(if rest.is_empty() { ".".to_string() } else { rest.into_owned() })
                });
                match relative {
                    Some(relative) => row.insert("cwd".into(), json!(relative)),
                    None => row.remove("cwd"),
                };
                chat
            })
            .collect(),
    )
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
