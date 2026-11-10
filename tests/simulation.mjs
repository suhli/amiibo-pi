// End-to-end check through the production HTTP, WebSocket and controller actor.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { randomBytes } from "node:crypto";

const dir = await mkdtemp(join(tmpdir(), "amiibo-simulation-"));
const token = randomBytes(24).toString("hex");
const server = spawn(
  process.argv[2] ?? "target/debug/amiibo-pi",
  ["--simulate"],
  {
    env: {
      ...process.env,
      AMIIBO_TOKEN: token,
      AMIIBO_BIND: "127.0.0.1:0",
      AMIIBO_DB: join(dir, "test.sqlite"),
    },
    stdio: ["ignore", "ignore", "pipe"],
  },
);
let ws;
const exited = once(server, "exit");
const timeout = (message) =>
  new Promise((_, reject) => {
    const t = setTimeout(() => reject(Error(message)), 8000);
    t.unref();
  });
try {
  const lines = createInterface({ input: server.stderr });
  const base = await Promise.race([
    (async () => {
      for await (const line of lines) {
        const port = /Listening at 127\.0\.0\.1:(\d+)/.exec(line);
        if (port) return "http://127.0.0.1:" + port[1];
      }
      throw Error("Server exited before binding");
    })(),
    timeout("Server startup timeout"),
  ]);
  async function api(path, method = "GET", body, expected = 200, headers = {}) {
    const r = await fetch(base + "/api/" + path, {
      method,
      body,
      headers: { Authorization: "Bearer " + token, ...headers },
    });
    assert.equal(r.status, expected, path);
    return r.json();
  }
  assert.equal((await fetch(base + "/api/snapshot")).status, 401);
  assert.equal((await fetch(base)).status, 200);
  ws = new WebSocket(base.replace("http:", "ws:") + "/api/events", [
    "amiibo-v1",
    "auth." + token,
  ]);
  let snapshot;
  const updates = new EventTarget();
  ws.onmessage = (e) => {
    snapshot = JSON.parse(e.data);
    updates.dispatchEvent(new Event("snapshot"));
  };
  async function wait(check) {
    while (!snapshot || !check(snapshot)) {
      await Promise.race([
        new Promise((resolve) =>
          updates.addEventListener("snapshot", resolve, { once: true }),
        ),
        timeout("Expected snapshot not received"),
      ]);
    }
  }
  await wait((s) => s.controller.link === "Idle" && s.controller.simulation);
  const tag = new Uint8Array(540);
  tag[0] = 4;
  tag[3] = 140;
  tag.set([15, 224, 241, 16, 255, 238], 10);
  tag.set([1, 0, 15], 520);
  tag[527] = 4;
  tag[528] = 95;
  assert.equal(
    (await api("amiibo?name=bad", "POST", new Uint8Array(540), 422)).code,
    "InvalidBackup",
  );
  assert.equal(
    (await api("amiibo?name=big", "POST", new Uint8Array(573), 413)).code,
    "TooLarge",
  );
  const { id } = await api("amiibo?name=Fixture", "POST", tag);
  await api("amiibo?name=duplicate", "POST", tag, 409);
  await api(
    "amiibo/" + id,
    "PATCH",
    JSON.stringify({ favorite: true, category: "Synthetic" }),
    200,
    { "Content-Type": "application/json" },
  );
  await api("amiibo/" + id + "/select", "POST");
  await api("nfc/arm", "POST", undefined, 409);
  await api("controller/pair", "POST");
  await wait((s) => s.controller.link === "Ready");
  await api("controller/press?button=A", "POST");
  await api("nfc/arm", "POST");
  await wait((s) => s.controller.scan === "Armed");
  assert.equal(snapshot.history.length, 0);
  await api("simulation/replay", "POST");
  await wait(
    (s) =>
      s.history[0]?.outcome === "Transferred" &&
      s.controller.scan === "Transferred",
  );
  assert.equal(snapshot.history[0].simulation, true);
  await api("nfc/arm", "POST");
  await api("nfc/cancel", "POST");
  await wait((s) => s.controller.scan === "Cancelled");
  for (let i = 0; i < 4; i++) {
    await api("nfc/arm", "POST");
    await api("controller/disconnect", "POST");
    await wait(
      (s) =>
        s.controller.link === "Disconnected" &&
        s.controller.armed_name === null,
    );
    await api("controller/reconnect", "POST");
    await wait((s) => s.controller.link === "Ready");
  }
  await api("amiibo/" + id, "DELETE");
  assert.equal((await api("amiibo")).length, 0);
  assert.equal((await api("history")).length, 6);
  console.log(
    "Simulation passed: HTTP auth, WebSocket, CRUD, host read, cancellation and four reconnects.",
  );
} finally {
  ws?.close();
  server.kill("SIGTERM");
  try {
    await Promise.race([exited, timeout("Shutdown timeout")]);
  } catch (e) {
    server.kill("SIGKILL");
    await exited;
    throw e;
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}
