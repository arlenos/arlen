// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The positive control for the shape our completion endpoint answers pi with.
//
// pi's openai-completions provider sends `stream: true` and there is no compat
// flag that turns it off, so an endpoint that answers one plain JSON object gets
// "Stream ended without finish_reason": the turn ends with `stopReason: "error"`
// and EMPTY content, after three silent auto-retries, with nothing on stderr.
// That looked exactly like a dead model for two boots, and the model was never
// dialled once.
//
// This drives a real pi against a stand-in socket in both shapes and asserts the
// difference, so the endpoint's `completion_to_sse` has a check outside its own
// unit tests - a Rust test proves the bytes are what we meant, this proves pi
// accepts them. It needs no model, no network and no VM, which is the point: the
// twenty-minute image loop is the wrong instrument for a question about a JSON
// shape.
//
// It waits for the outcome rather than sleeping a fixed span. A 20s sleep per
// turn passed on an idle laptop and went red inside a full pre-commit run, where
// pi had not finished starting when the clock ran out: the gate was measuring the
// machine's load, not pi's behaviour, and it failed an unrelated commit.
//
// Skips (exit 0) when pi or the node runtime is absent, the way the e2e gate
// tests do - this is a developer control, not a gate that fails on a fresh clone.
//
//     node dev/scripts/test-pi-completion-shape.mjs

import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync, mkdirSync, rmSync, existsSync } from "node:fs";
import { tmpdir, homedir } from "node:os";
import { join } from "node:path";

const NODE_BIN = process.env.ARLEN_PI_NODE_RUNTIME
  ? join(process.env.ARLEN_PI_NODE_RUNTIME, "bin/node")
  : join(homedir(), ".local/share/arlen-node22/bin/node");
const PI_CLI = join(
  process.env.ARLEN_PI_INSTALL || join(homedir(), "Repositories/pi"),
  "packages/coding-agent/dist/cli.js",
);

if (!existsSync(NODE_BIN) || !existsSync(PI_CLI)) {
  console.log(`  skip  pi or its node runtime is absent (${NODE_BIN}, ${PI_CLI})`);
  process.exit(0);
}

let failures = 0;
function check(name, ok) {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`);
  if (!ok) failures++;
}

/// The answer text, so a run that never reached the stand-in cannot produce it.
const ANSWER = "the stand-in answered";

/// One non-streaming OpenAI completion - the shape the echo provider returns and
/// the daemon used to write back verbatim.
const AS_OBJECT = JSON.stringify({
  id: "echo",
  object: "chat.completion",
  model: "echo",
  choices: [
    { index: 0, message: { role: "assistant", content: ANSWER }, finish_reason: "stop" },
  ],
});

/// The same answer as the frames `completion_to_sse` emits.
const AS_FRAMES =
  `data: ${JSON.stringify({
    id: "echo",
    object: "chat.completion.chunk",
    created: 0,
    model: "echo",
    choices: [{ index: 0, delta: { role: "assistant", content: ANSWER }, finish_reason: null }],
  })}\n\n` +
  `data: ${JSON.stringify({
    id: "echo",
    object: "chat.completion.chunk",
    created: 0,
    model: "echo",
    choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
  })}\n\n` +
  "data: [DONE]\n\n";

/// Run one pi turn against a stand-in that answers with `body`, and report what
/// pi made of it. stdin is held OPEN throughout: closing it makes pi exit before
/// its first retry, which is what made the original VM journal look like a turn
/// that simply stopped rather than one that errored and retried.
///
/// `settled` says when the turn has shown what it is going to show, so a loaded
/// machine gets the time it needs and an idle one is not made to wait for it.
/// The deadline is the give-up, not the measurement.
async function pi_turn(body, contentType, settled, deadline_ms = 90000) {
  const dir = mkdtempSync(join(tmpdir(), "pi-shape-"));
  const sock = join(dir, "proxy.sock");
  mkdirSync(join(dir, ".pi/agent"), { recursive: true });
  writeFileSync(
    join(dir, ".pi/agent/models.json"),
    JSON.stringify({
      providers: {
        arlen: {
          baseUrl: "http://localhost:11434/v1",
          api: "openai-completions",
          apiKey: "control-token",
          compat: { supportsDeveloperRole: false, supportsReasoningEffort: false },
          models: [{ id: "qwen2.5:7b" }],
        },
      },
    }),
  );

  let dials = 0;
  const srv = createServer((req, res) => {
    dials++;
    req.on("data", () => {});
    req.on("end", () => {
      res.writeHead(200, { "content-type": contentType });
      res.end(body);
    });
  });
  await new Promise((r) => srv.listen(sock, r));

  const events = [];
  const pi = spawn(
    NODE_BIN,
    [PI_CLI, "--mode", "rpc", "--provider", "arlen", "--model", "qwen2.5:7b"],
    {
      env: { ...process.env, HOME: dir, ARLEN_AI_PROXY_SOCKET: sock },
      stdio: ["pipe", "pipe", "pipe"],
    },
  );
  pi.stdout.on("data", (d) => events.push(String(d)));
  pi.stdin.write(JSON.stringify({ type: "prompt", message: "say hello" }) + "\n");

  const read = () => {
    const text = events.join("");
    return {
      dials,
      answered: text.includes(ANSWER),
      errored: text.includes('"stopReason":"error"'),
      retries: (text.match(/auto_retry_start/g) || []).length,
    };
  };

  const started = Date.now();
  while (!settled(read()) && Date.now() - started < deadline_ms) {
    await new Promise((r) => setTimeout(r, 250));
  }
  // A turn that has just said one thing may be about to say another - an error
  // after an answer would change the verdict - so let the tail land before the
  // checks read it.
  await new Promise((r) => setTimeout(r, 1500));
  const outcome = read();

  pi.kill();
  srv.close();
  rmSync(dir, { recursive: true, force: true });
  return outcome;
}

// The good shape is settled once the answer is through.
const frames = await pi_turn(AS_FRAMES, "text/event-stream", (s) => s.answered);
check("pi dials the stand-in at all", frames.dials > 0);
check("frames: pi delivers the answer", frames.answered);
check("frames: no turn ends in error", !frames.errored);
check("frames: no retries were needed", frames.retries === 0);

// The bad shape is settled once it has errored AND retried at least once, which
// is the whole claim: pi treats a plain JSON object as a turn that failed.
const object = await pi_turn(
  AS_OBJECT,
  "application/json",
  (s) => s.errored && s.retries > 0 && s.dials > 1,
);
check("object: pi delivers NOTHING", !object.answered);
check("object: the turn ends in error", object.errored);
check("object: pi retries silently", object.retries > 0);
check("object: it dialled every time, so the model was reachable", object.dials > 1);

console.log(failures ? `\n${failures} failure(s)` : "\nboth shapes behave as the endpoint assumes");
process.exit(failures ? 1 : 0);
