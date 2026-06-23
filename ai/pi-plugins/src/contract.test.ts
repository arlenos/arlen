import { test } from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as net from "node:net";
import {
  calls,
  ContractClient,
  encodeFrame,
  FrameReader,
  MAX_FRAME,
  readSessionToken,
  type ContractCall,
  type Reply,
} from "./contract.js";

test("encodeFrame writes a 4-byte LE length then the UTF-8 JSON body", () => {
  const frame = encodeFrame({ a: 1 });
  const body = Buffer.from(JSON.stringify({ a: 1 }), "utf8");
  assert.equal(frame.readUInt32LE(0), body.length);
  assert.deepEqual(frame.subarray(4), body);
});

test("FrameReader reassembles split chunks and yields multiple frames", () => {
  const r = new FrameReader();
  const f1 = encodeFrame({ n: 1 });
  const f2 = encodeFrame({ n: 2 });
  // Split f1 across two pushes, then deliver f2 whole in the same buffer.
  assert.deepEqual(r.push(f1.subarray(0, 3)), []);
  const got = r.push(Buffer.concat([f1.subarray(3), f2]));
  assert.deepEqual(got, [{ n: 1 }, { n: 2 }]);
});

test("FrameReader rejects an oversized length prefix", () => {
  const r = new FrameReader();
  const len = Buffer.alloc(4);
  len.writeUInt32LE(MAX_FRAME + 1, 0);
  assert.throws(() => r.push(len), /frame too large/);
});

test("the authorize ContractCall matches the daemon's pinned envelope", () => {
  const call: ContractCall = { token: "tok-1", call: calls.authorize("graph.write", { cypher: "CREATE (n)" }, false) };
  const decoded = new FrameReader().push(encodeFrame(call))[0];
  assert.deepEqual(decoded, {
    token: "tok-1",
    call: {
      call: "authorize",
      tool_name: "graph.write",
      tool_input: { cypher: "CREATE (n)" },
      external_triggered: false,
    },
  });
});

test("readSessionToken throws without the env var, reads the file with it", () => {
  const saved = process.env.ARLEN_AI_ENGINE_TOKEN_FILE;
  try {
    delete process.env.ARLEN_AI_ENGINE_TOKEN_FILE;
    assert.throws(() => readSessionToken(), /ARLEN_AI_ENGINE_TOKEN_FILE/);
    const dir = mkdtempSync(join(tmpdir(), "arlen-pi-tok-"));
    const path = join(dir, "token");
    writeFileSync(path, "s3cr3t\n");
    process.env.ARLEN_AI_ENGINE_TOKEN_FILE = path;
    assert.equal(readSessionToken(), "s3cr3t");
  } finally {
    if (saved === undefined) delete process.env.ARLEN_AI_ENGINE_TOKEN_FILE;
    else process.env.ARLEN_AI_ENGINE_TOKEN_FILE = saved;
  }
});

test("ContractClient sends a framed call and parses the framed reply", async () => {
  // A mock daemon: read one framed ContractCall, assert its shape, reply with a
  // framed AuthorizeDecision (the full round-trip, no real daemon).
  const dir = mkdtempSync(join(tmpdir(), "arlen-pi-sock-"));
  const sockPath = join(dir, "ai-engine.sock");
  let seen: ContractCall | undefined;

  const server = net.createServer((conn) => {
    const reader = new FrameReader();
    conn.on("data", (chunk: Buffer) => {
      const frames = reader.push(chunk);
      if (frames.length > 0) {
        seen = frames[0] as ContractCall;
        const reply: Reply = { reply: "authorize", decision: "allow" };
        conn.write(encodeFrame(reply));
      }
    });
  });
  server.listen(sockPath);
  await once(server, "listening");

  try {
    const client = await ContractClient.connect(sockPath, "tok-xyz");
    const reply = await client.call(calls.authorize("graph.read", { query: "x" }, true));
    client.close();

    assert.deepEqual(reply, { reply: "authorize", decision: "allow" });
    assert.ok(seen);
    assert.equal(seen.token, "tok-xyz");
    assert.deepEqual(seen.call, {
      call: "authorize",
      tool_name: "graph.read",
      tool_input: { query: "x" },
      external_triggered: true,
    });
  } finally {
    server.close();
  }
});
