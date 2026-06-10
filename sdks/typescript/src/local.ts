/**
 * Local (CRW_LOCAL) subprocess transport: speaks MCP JSON-RPC to a `crw-mcp`
 * binary over stdio. Mirrors the Python SDK's subprocess mode.
 *
 * v1 finds the binary via the `CRW_BINARY` env var or on `PATH`; auto-download
 * (as the Python SDK does) is a fast-follow.
 */

import { type ChildProcessByStdio, spawn } from "node:child_process";
import type { Readable, Writable } from "node:stream";
import { CrwApiError, CrwBinaryNotFoundError, CrwError } from "./errors.js";

type McpProc = ChildProcessByStdio<Writable, Readable, null>;
import type { Json } from "./types.js";

const BINARY_NAME = process.platform === "win32" ? "crw-mcp.exe" : "crw-mcp";

interface Pending {
  resolve: (v: Json) => void;
  reject: (e: Error) => void;
}

export class LocalTransport {
  private proc: McpProc | null = null;
  private nextId = 0;
  private pending = new Map<number, Pending>();
  private buffer = "";

  private resolveBinary(): string {
    const env = process.env.CRW_BINARY;
    if (env) return env;
    // Rely on PATH resolution by spawning the bare name; if it ENOENTs the
    // error handler surfaces a clear install hint.
    return BINARY_NAME;
  }

  private ensureProcess(): McpProc {
    if (this.proc && this.proc.exitCode === null) return this.proc;
    const bin = this.resolveBinary();
    const proc = spawn(bin, [], { stdio: ["pipe", "pipe", "ignore"] });
    proc.on("error", (err: NodeJS.ErrnoException) => {
      const failure =
        err.code === "ENOENT"
          ? new CrwBinaryNotFoundError(
              `crw-mcp binary not found on PATH. Install it (e.g. \`npm i -g crw-mcp\` or ` +
                `\`cargo install crw-mcp\`) or set CRW_BINARY to its path.`,
            )
          : new CrwError(`crw-mcp failed to start: ${err.message}`);
      for (const p of this.pending.values()) p.reject(failure);
      this.pending.clear();
    });
    proc.stdout.setEncoding("utf8");
    proc.stdout.on("data", (chunk: string) => this.onData(chunk));
    proc.on("exit", () => {
      for (const p of this.pending.values()) p.reject(new CrwError("crw-mcp process closed unexpectedly"));
      this.pending.clear();
    });
    this.proc = proc;
    return proc;
  }

  private onData(chunk: string): void {
    this.buffer += chunk;
    let idx: number;
    while ((idx = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, idx).trim();
      this.buffer = this.buffer.slice(idx + 1);
      if (!line) continue;
      let msg: Json;
      try {
        msg = JSON.parse(line) as Json;
      } catch {
        continue;
      }
      const id = msg.id as number | undefined;
      if (id === undefined || !this.pending.has(id)) continue;
      const p = this.pending.get(id)!;
      this.pending.delete(id);
      if (msg.error) {
        const err = msg.error as { message?: string };
        p.reject(new CrwApiError(err.message ?? JSON.stringify(msg.error)));
      } else {
        p.resolve((msg.result as Json) ?? {});
      }
    }
  }

  private jsonrpc(method: string, params: Json): Promise<Json> {
    const proc = this.ensureProcess();
    const id = ++this.nextId;
    return new Promise<Json>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
  }

  async toolCall(name: string, args: Json): Promise<Json> {
    const result = await this.jsonrpc("tools/call", { name, arguments: args });
    const content = (result.content as Array<{ text?: string }> | undefined)?.[0];
    if (!content) throw new CrwError(`Empty response from ${name}`);
    if (result.isError) throw new CrwApiError(content.text ?? "Unknown error");
    return JSON.parse(content.text ?? "{}") as Json;
  }

  close(): void {
    if (this.proc && this.proc.exitCode === null) {
      this.proc.stdin.end();
      this.proc.kill();
    }
    this.proc = null;
  }
}
