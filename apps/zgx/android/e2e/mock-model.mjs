// A stand-in model for the e2e suite (OpenAI chat completions, streamed):
// every request's last user message is appended to $MOCK_LOG as a JSON line,
// so the suite reads exactly what reached the model, and the reply is long
// (40 paragraphs) so the transcript scrolls. No real model is used.
import http from "node:http";
import fs from "node:fs";
const port = +process.argv[2];
const log = process.env.MOCK_LOG;
const paras = [];
for (let i = 1; i <= 40; i++) paras.push(`Paragraph ${i}: The quick brown fox jumps over the lazy dog while the composer waits below. Select a word here to copy it, scroll up and down, and watch the keyboard stay put.`);
const reply = "# Long mock reply\n\n" + paras.join("\n\n");
http.createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    if (!req.url.includes("chat/completions")) {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ data: [{ id: "long", object: "model" }] }));
      return;
    }
    try {
      const msgs = JSON.parse(body).messages || [];
      const last = [...msgs].reverse().find((m) => m.role === "user");
      const text = typeof last?.content === "string" ? last.content : (last?.content || []).filter((p) => p.type === "text").map((p) => p.text).join("");
      if (log) fs.appendFileSync(log, JSON.stringify({ at: Date.now(), text }) + "\n");
    } catch {}
    res.writeHead(200, { "content-type": "text/event-stream" });
    const id = "mock-" + Date.now();
    const chunk = (delta, finish) => res.write("data: " + JSON.stringify({ id, object: "chat.completion.chunk", created: Math.floor(Date.now() / 1000), model: "long", choices: [{ index: 0, delta, finish_reason: finish ?? null }] }) + "\n\n");
    chunk({ role: "assistant", content: "" });
    for (let i = 0; i < reply.length; i += 400) chunk({ content: reply.slice(i, i + 400) });
    chunk({}, "stop");
    res.write("data: " + JSON.stringify({ id, object: "chat.completion.chunk", choices: [], usage: { prompt_tokens: 10, completion_tokens: 500, total_tokens: 510 } }) + "\n\n");
    res.end("data: [DONE]\n\n");
  });
}).listen(port, "127.0.0.1", () => console.log("mock model on " + port));
