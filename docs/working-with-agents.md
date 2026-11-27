# Working with agents

For people who use Zend with agents that delegate. The tools behind it are in [mcp.md](mcp.md).

## Chats that start chats
An agent in a chat can hand work to other agents. What it starts shows in different places:

| Kind | Where you find it | For |
|---|---|---|
| Subagent | under the chat, in its Subagents list and in the transcript | a piece of delegated work |
| Child session | its own row in the sidebar, nested under the chat, with its own page | a larger piece you may want to follow or talk to; kept when the parent is archived or deleted |
| Side chat | beside the chat | a conversation of your own on the side |

A child can run on another harness or model than its parent (a Pi chat can hand work to Claude Code or Codex). Delegation is one level deep: a child starts no chats.

You can open any child and write to it like any chat.

## What a Zend event is
The parent does not wait for its children. When a child finishes a turn, fails, is interrupted or asks something, Zend puts a line into the parent:

```
[Zend event: chat <title> (<id>) completed. Last reply: … Read more: read_chat <id>.]
```

The line comes from Zend, not from you and not from an agent. A parent that is working gets it at once; a parent at rest starts a turn with it; a parent waiting on you gets it when that turn ends. Children finishing within two seconds arrive together.

So a lead agent typically ends its turn after starting children and continues by itself as they report. You don't have to nudge it.

## When a child asks something
A child that needs an answer shows as waiting for input. Its parent is told and may answer it (`respond_to_input`), or you answer in the child's own page.

## Visuals in replies
An agent can show a chart, table, diagram or 3D view above its reply ([visuals.md](visuals.md)). You can use its tabs, toggles and sliders; that changes only what you see.

Today a visual cannot take your answers. Being built ([roadmap.md](roadmap.md)): fields, checklists and choices in a visual; a button that appends your answers to the message box, where you edit and send them; and the agent updating the same visual while you are still answering, without overwriting what you typed. Until you send, what you typed is your draft.

## Telling your agents how to work
Plugins with the permission `session.instructions` put text into your agents' system prompt, for every chat, child sessions and subagents included. Use it for standing rules: how to delegate, which model for what, when to show a visual.

- Each such plugin shows its text in Settings, on its page.
- Works for chats on Pi. Chats on other harnesses run without it for now.
- 8 KB a chat for each plugin. A change reaches a running chat at its next turn and costs it its prompt cache once.

## Editing a prompt, forking from a message
Hover a message to see its actions beside Copy.

- **Edit** (your own prompts): the prompt opens as an editor in place. Change the words, take off a file or picture with its ×, then Save & send (⌘Enter), or Cancel (Escape). If anything is below it, Zend asks once: everything below goes, for good. Files the agent changed on disk stay as they are; undo those yourself or ask the agent. The agent then works from a fresh session that gets the conversation up to your edited prompt. While the agent is still replying you can't send an edit: stop it or wait. An edit that can't go (a turn started meanwhile, Zend refused it) stays open in its editor with the reason; if the prompt is already removed and only the send fails, the words go into the message box after whatever you had there.
- **Fork from here** (any prompt or reply): opens a side chat. From a reply it holds the conversation up to and including that reply; from a prompt, everything before it, with the prompt's words in the side chat's message box to change and send. The chat you forked from stays as it is.

## Naming branches and worktrees
The composer's Branch chip names where a chat works.

- **New chat, local checkout:** Branch → New branch…, type the name. The first send makes it off the ref you picked (`git switch -c`) in the project folder, then starts the chat there. The dialog names the chats already working in that folder: their checkout switches too. A name git refuses or a branch that exists is said there; nothing is sent. Pick another ref afterwards that is checked out in a worktree of its own, and the name is dropped: it was given for the project folder.
- **New chat, worktree:** pick Checkout → Worktree, then Branch → Name the worktree…. The name is the folder and the branch; give the branch apart if it should differ. Leave it empty for a generated name.
- **Open chat:** click its branch chip. Rename branch… renames the branch checked out there; Rename worktree… moves the worktree's folder and renames its branch, and the agent goes on with its history in a fresh session; Switch branch (local checkout) checks out another branch, stopping with git's reason when uncommitted changes are in the way. All chats in that folder follow, and the dialog lists them. Not while a turn runs in that folder: stop it or wait. A message sent there while a rename is under way, from a new chat too, waits for it and then runs in the folder as it is. A branch name git can't keep beside another (`topic` and `topic/sub`) is refused before anything moves; if git refuses the branch after the folder moved, the folder goes back, and if it can't, its chats follow it and the error says where it is.

## Auto access

The access picker has four modes: Supervised, Auto-accept edits, Auto and Full access. Auto hands the approval to the agent's own reviewer: it lets safe actions through and asks you about risky ones. Only Claude Code and Codex have such a reviewer, so only they offer Auto.

If Auto can't be honoured for a chat (the agent has no Auto mode), Zend never quietly runs it looser: Send is held and the composer says why. Pick another mode or agent to go on. A reviewer's verdict shows as a shield on the tool's row; text a tool prints can't pose as one.

## Usage and accounts
Limits shown in Zend are read from each provider with your sign-in. If a provider's sign-in has expired, readings stop and go stale without the chats failing: see [troubleshooting.md](troubleshooting.md).
