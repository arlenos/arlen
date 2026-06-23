// The Arlen AI-engine contract, from the engine (pi) side.
//
// The thin pi plugins call the ai-engine-daemon over its Unix socket with the
// wire the daemon defines (`ai-engine-contract`): a 4-byte little-endian length
// prefix then that many UTF-8 bytes of a JSON `ContractCall`; the daemon replies
// with the same framing carrying a `Reply`. These types mirror the daemon's
// serde shapes EXACTLY (the daemon pins them with a conformance test), so a
// plugin's hand-built JSON is what the daemon expects. The daemon authenticates
// the calling process via SO_PEERCRED + the session token (read from the file at
// `ARLEN_AI_ENGINE_TOKEN_FILE`); the token is the only secret the plugin holds.

import { once } from "node:events";
import { readFileSync } from "node:fs";
import * as net from "node:net";

/** How much of the graph a session may read (daemon-enforced; prompt context). */
export type ReadTier = "none" | "minimal" | "standard" | "extended" | "full";

/** The S17/S18 screening verdict on reported content. */
export type ScreenVerdict = "clean" | "warn" | "block";

/** A contract-level failure code (the daemon's `ContractError`, snake_case). */
export type ContractError =
  | "unknown_token"
  | "pid_mismatch"
  | "unknown_tool"
  | "invalid_arguments"
  | "permission_denied"
  | "execution_failed"
  | "unavailable"
  | "internal";

/** The daemon's verdict on an `Authorize` (internally tagged on `decision`). */
export type AuthorizeDecision =
  | { decision: "allow" }
  | { decision: "deny"; reason: string }
  | { decision: "modify"; args: unknown }
  | { decision: "confirm"; prompt: string };

/** The outcome of an `Execute` (internally tagged on `outcome`). */
export type ExecuteOutcome =
  | { outcome: "ok"; result: unknown }
  | { outcome: "error"; code: ContractError; message: string };

/** The screen ack for a `Report`. */
export interface ReportAck {
  screen: ScreenVerdict;
}

/** The verb being asked of the daemon (internally tagged on `call`). */
export type Call =
  | { call: "authorize"; tool_name: string; tool_input: unknown; external_triggered: boolean }
  | { call: "execute"; tool_name: string; tool_input: unknown }
  | { call: "report"; tool_name: string; tool_call_id: string; result: unknown; is_error: boolean }
  | { call: "end_session" };

/** The engine-to-daemon message: the session token plus one `Call`. */
export interface ContractCall {
  token: string;
  call: Call;
}

/** The daemon's reply (internally tagged on `reply`), matching the call. */
export type Reply =
  | ({ reply: "authorize" } & AuthorizeDecision)
  | ({ reply: "execute" } & ExecuteOutcome)
  | ({ reply: "report" } & ReportAck)
  | { reply: "ack" }
  | { reply: "error"; code: ContractError };

/** The largest reply frame the client will accept (a defensive cap on a hostile
 *  length prefix; real replies are small). */
export const MAX_FRAME = 16 * 1024 * 1024;

/** Encode a value as one length-prefixed frame: 4-byte LE length + UTF-8 JSON. */
export function encodeFrame(value: unknown): Buffer {
  const body = Buffer.from(JSON.stringify(value), "utf8");
  const len = Buffer.alloc(4);
  len.writeUInt32LE(body.length, 0);
  return Buffer.concat([len, body]);
}

/** Accumulates socket bytes and yields each complete frame's decoded JSON. A
 *  length over `MAX_FRAME` throws (fail-closed on a hostile prefix). */
export class FrameReader {
  private buf: Buffer = Buffer.alloc(0);

  push(chunk: Buffer): unknown[] {
    this.buf = this.buf.length === 0 ? chunk : Buffer.concat([this.buf, chunk]);
    const out: unknown[] = [];
    for (;;) {
      if (this.buf.length < 4) break;
      const len = this.buf.readUInt32LE(0);
      if (len > MAX_FRAME) throw new Error(`contract reply frame too large: ${len} > ${MAX_FRAME}`);
      if (this.buf.length < 4 + len) break;
      const body = this.buf.subarray(4, 4 + len);
      out.push(JSON.parse(body.toString("utf8")));
      this.buf = this.buf.subarray(4 + len);
    }
    return out;
  }
}

/** Read the session token from `ARLEN_AI_ENGINE_TOKEN_FILE` (the 0600 file the
 *  daemon wrote for this run). Throws if the env var is unset or unreadable. */
export function readSessionToken(): string {
  const path = process.env.ARLEN_AI_ENGINE_TOKEN_FILE;
  if (!path) throw new Error("ARLEN_AI_ENGINE_TOKEN_FILE is not set");
  return readFileSync(path, "utf8").trim();
}

/** The contract socket path, from `ARLEN_AI_ENGINE_SOCKET`. */
export function socketPath(): string {
  const path = process.env.ARLEN_AI_ENGINE_SOCKET;
  if (!path) throw new Error("ARLEN_AI_ENGINE_SOCKET is not set");
  return path;
}

/** Builders for the four calls (so a plugin never hand-shapes the JSON). */
export const calls = {
  authorize(toolName: string, toolInput: unknown, externalTriggered: boolean): Call {
    return { call: "authorize", tool_name: toolName, tool_input: toolInput, external_triggered: externalTriggered };
  },
  execute(toolName: string, toolInput: unknown): Call {
    return { call: "execute", tool_name: toolName, tool_input: toolInput };
  },
  report(toolName: string, toolCallId: string, result: unknown, isError: boolean): Call {
    return { call: "report", tool_name: toolName, tool_call_id: toolCallId, result, is_error: isError };
  },
  endSession(): Call {
    return { call: "end_session" };
  },
};

/** A sequential request/reply client over the daemon's contract socket: connect
 *  once, then `call()` one verb at a time (the gate plugin authorizes one tool
 *  call at a time). The session token is attached to every call. */
export class ContractClient {
  private constructor(
    private readonly socket: net.Socket,
    private readonly token: string,
    private readonly reader: FrameReader,
  ) {}

  /** Connect to `path` (default: `ARLEN_AI_ENGINE_SOCKET`) with the session
   *  token (default: read from `ARLEN_AI_ENGINE_TOKEN_FILE`). */
  static async connect(path: string = socketPath(), token: string = readSessionToken()): Promise<ContractClient> {
    const socket = net.connect(path);
    await once(socket, "connect");
    return new ContractClient(socket, token, new FrameReader());
  }

  /** Send one `Call` and resolve with the daemon's `Reply`. Rejects if the
   *  connection closes before a full reply, or a frame is malformed/oversized. */
  async call(call: Call): Promise<Reply> {
    const frame = encodeFrame({ token: this.token, call } satisfies ContractCall);
    return new Promise<Reply>((resolve, reject) => {
      const onData = (chunk: Buffer) => {
        let frames: unknown[];
        try {
          frames = this.reader.push(chunk);
        } catch (err) {
          cleanup();
          reject(err);
          return;
        }
        if (frames.length > 0) {
          cleanup();
          resolve(frames[0] as Reply);
        }
      };
      const onClose = () => {
        cleanup();
        reject(new Error("contract connection closed before a reply"));
      };
      const onError = (err: Error) => {
        cleanup();
        reject(err);
      };
      const cleanup = () => {
        this.socket.off("data", onData);
        this.socket.off("close", onClose);
        this.socket.off("error", onError);
      };
      this.socket.on("data", onData);
      this.socket.on("close", onClose);
      this.socket.on("error", onError);
      this.socket.write(frame);
    });
  }

  /** Close the connection. */
  close(): void {
    this.socket.end();
  }
}
