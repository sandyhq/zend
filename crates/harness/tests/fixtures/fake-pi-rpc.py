#!/usr/bin/env python3
"""Native RPC session persistence fixture for engine dispatch tests.

Keeps a Pi-shaped session file: a header, then one entry per line (each
prompt writes its user message, a tool result and the reply). `--fork`,
RPC `fork` and `get_entries` follow Pi 1.5.2, and so does reading a session
file: one whose last line has no newline gets one (session-manager.js:367).
Files in the working folder change it: `fail-fork` makes RPC `fork` fail,
`die-after-fork` makes `--fork` exit once it wrote its copy,
`report-file` makes a `--fork` Pi name the file it holds as its session,
`fail-open` makes Pi exit on a `--session` file that names a parent, and
`open-other` makes it answer for another session there. Its session
stats (`/session`) are `reply:/session`, and kept nowhere."""
import json
import os
from pathlib import Path
import sys
import threading
import time
import uuid

if '--version' in sys.argv:
    print('0.85.1')
    sys.exit(0)

def arg(flag):
    return sys.argv[sys.argv.index(flag) + 1] if flag in sys.argv else None

def read(path):
    raw = Path(path).read_bytes()
    lines = [json.loads(l) for l in raw.decode().splitlines() if l.strip()]
    if raw and not raw.endswith(b'\n') and lines and lines[0].get('type') == 'session':
        with open(path, 'ab') as f:
            f.write(b'\n')
    return lines[0], lines[1:]

def create(parent, entries):
    sid = str(uuid.uuid4())
    folder = Path(arg('--session-dir') or Path.cwd())
    folder.mkdir(parents=True, exist_ok=True)
    file = folder / (sid + '.jsonl')
    header = {'type': 'session', 'version': 3, 'id': sid, 'cwd': str(Path.cwd())}
    if parent:
        header['parentSession'] = str(parent)
    file.write_text(''.join(json.dumps(e) + '\n' for e in [header] + entries))
    return sid, file

resumed = '--session' in sys.argv
entries = []
if resumed:
    path = Path(arg('--session'))
    header, entries = read(path)
    session = header['id']
    if header.get('parentSession') and Path('fail-open').exists():
        sys.exit(1)
    if header.get('parentSession') and Path('open-other').exists():
        session = str(uuid.uuid4())
elif '--fork' in sys.argv:
    source = Path(arg('--fork')).resolve()
    _, entries = read(source)
    session, path = create(source, entries)
    if Path('die-after-fork').exists():
        os._exit(3)
elif '--no-session' in sys.argv:
    session, path = str(uuid.uuid4()), Path.cwd() / 'unsaved.jsonl'
else:
    session, path = create(None, [])

def emit(frame):
    print(json.dumps(frame), flush=True)

def append(entry):
    entry['id'] = uuid.uuid4().hex[:8]
    entry['parentId'] = entries[-1]['id'] if entries else None
    entries.append(entry)
    if '--no-session' not in sys.argv:
        with open(path, 'a') as f:
            f.write(json.dumps(entry) + '\n')

def crash():
    time.sleep(0.15)
    os._exit(7)

dialog = None
for line in sys.stdin:
    req = json.loads(line)
    kind = req['type']
    if kind == 'prompt' and req['message'] == '/question':
        dialog = req
        emit({'type':'extension_ui_request','id':'question','method':'select','title':'Choose','options':['first','second'],'timeout':200})
        continue
    if kind == 'extension_ui_response':
        assert req.get('cancelled') is True
        emit({'id':dialog['id'],'type':'response','command':'prompt','success':True})
        continue
    data = {}
    success = True
    error = 'session must be resumed'
    if kind == 'get_state':
        named = Path('report-file').read_text().strip() if '--fork' in sys.argv and Path('report-file').exists() else str(path)
        data = {'sessionId': session, 'sessionFile': named, 'isStreaming': False, 'isCompacting': False, 'model': {'id': 'mock'}}
    if kind == 'get_session_stats':
        data = 'reply:/session'
    if kind == 'get_commands':
        data = {'commands': []}
    if kind == 'get_entries':
        data = {'entries': entries, 'leafId': entries[-1]['id'] if entries else None}
    if kind == 'fork':
        # Pi's createBranchedSession: the path from the root to the
        # entry's parent, other branches left out.
        by_id = {e['id']: e for e in entries}
        target = by_id.get(req.get('entryId'))
        if Path('fail-fork').exists() or target is None or target.get('message', {}).get('role') != 'user':
            success, error = False, 'Invalid entry ID for forking'
        else:
            branch, at = [], target.get('parentId')
            while at:
                branch.append(by_id[at])
                at = by_id[at].get('parentId')
            entries = branch[::-1]
            session, path = create(path, entries)
            data = {'text': 'forked', 'cancelled': False}
    if kind == 'prompt' and req['message'] == 'require-resume' and not resumed:
        success = False
    emit({'id': req.get('id'), 'type': 'response', 'command': kind, 'success': success, 'data': data, 'error': error})
    if kind == 'prompt' and success:
        text = 'reply:' + req['message']
        append({'type': 'message', 'message': {'role': 'user', 'content': [{'type': 'text', 'text': req['message']}]}})
        append({'type': 'message', 'message': {'role': 'toolResult', 'content': [{'type': 'text', 'text': 'tool ran'}]}})
        append({'type': 'message', 'message': {'role': 'assistant', 'content': [{'type': 'text', 'text': text}]}})
        emit({'type': 'agent_start'})
        emit({'type': 'message_start', 'message': {'role': 'user'}})
        emit({'type': 'message_end', 'message': {'role': 'assistant', 'content': [{'type': 'text', 'text': text}], 'stopReason': 'stop'}})
        emit({'type': 'agent_end'})
        emit({'type': 'agent_settled'})
        if req['message'] == 'idle-crash':
            threading.Thread(target=crash, daemon=True).start()
