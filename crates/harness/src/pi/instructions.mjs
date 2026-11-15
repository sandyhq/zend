// Plugins' instructions for this chat's agent (Zend's session.instructions),
// each in full in a system-prompt section of its own (plugin-<id>), never
// through MCP instructions. Read again before every agent turn: Zend rewrites
// the file when they change, and pi diffs the sections into the live prompt
// (and replays them after a compaction). No global settings are modified.
import { readFileSync } from "node:fs";

export default function (pi) {
  const path = process.env.ZEND_PI_INSTRUCTIONS;
  let shown = [];
  pi.on("before_agent_start", event => {
    const sections = event.systemPromptOptions?.sections;
    if (!sections) return;
    let list = [];
    try {
      list = JSON.parse(readFileSync(path, "utf8"));
    } catch {
      list = [];
    }
    for (const name of shown) delete sections[name];
    shown = [];
    for (const item of Array.isArray(list) ? list : []) {
      if (typeof item?.section !== "string" || typeof item?.text !== "string" || !item.text) continue;
      sections[item.section] = item.text;
      shown.push(item.section);
    }
  });
}
