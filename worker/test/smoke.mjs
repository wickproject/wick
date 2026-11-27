// Runs the worker's telemetry and feedback endpoints against an in-memory
// KV and fake Cloudflare requests. No dependencies: `node worker/test/smoke.mjs`.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// Workers accept `new Response("", { status: 204 })`; Node's undici rejects
// a body on 204, so drop it the way the Workers runtime effectively does.
const NodeResponse = globalThis.Response;
globalThis.Response = class extends NodeResponse {
  constructor(body, init) { super(init && init.status === 204 ? null : body, init); }
};

const worker = (await import(new URL("../src/index.js", import.meta.url))).default;
const cases = JSON.parse(readFileSync(new URL("./scrub-cases.json", import.meta.url), "utf8"));

function makeEnv({ limiterAllows = true } = {}) {
  const store = new Map();
  return {
    store,
    SUBSCRIPTIONS: {
      async get(k) { return store.has(k) ? store.get(k) : null; },
      async put(k, v) { store.set(k, v); },
      async list({ prefix }) {
        return { keys: [...store.keys()].filter(k => k.startsWith(prefix)).map(name => ({ name })), list_complete: true };
      },
    },
    FEEDBACK_LIMITER: { async limit() { return { success: limiterAllows }; } },
  };
}

function req(path, body, country) {
  const r = new Request("https://releases.getwick.dev" + path, body
    ? { method: "POST", body: JSON.stringify(body), headers: { "Content-Type": "application/json", "cf-connecting-ip": "203.0.113.7" } }
    : {});
  Object.defineProperty(r, "cf", { value: country ? { country } : undefined });
  return r;
}

const today = new Date().toISOString().split("T")[0];
const feedbackRecords = env => [...env.store].filter(([k]) => k.startsWith("fb:")).map(([, v]) => JSON.parse(v));

// Events: country, tool and retry counts; unknown tools dropped.
{
  const env = makeEnv();
  const ev = (o, c) => worker.fetch(req("/v1/events", { host: "comparis.ch", strategy: "cronet-blocked", ok: false, status: 403, timing_ms: 900, ...o }, c), env);
  await ev({ tool: "wick_fetch" }, "CH");
  await ev({ tool: "wick_fetch", retry: true }, "CH");
  await ev({ tool: "wick_fetch", retry: true, ok: true, status: 200 }, "DE");
  await ev({ tool: "not_a_tool" });
  const v = JSON.parse(env.store.get(`evt:${today}:comparis.ch:cronet-blocked`));
  assert.equal(v.fetches, 4);
  assert.deepEqual(v.country_dist, { CH: 2, DE: 1, XX: 1 });
  assert.deepEqual(v.tool_dist, { wick_fetch: 3 });
  assert.equal(v.retries, 2);
  assert.equal(v.retry_successes, 1);

  const summary = await (await worker.fetch(req("/v1/stats/summary"), env)).json();
  const row = summary.rows.find(r => r.host === "comparis.ch");
  assert.equal(row.country_dist, undefined, "public summary must not expose per-host countries");
  assert.deepEqual(row.tool_dist, { wick_fetch: 3 });
  assert.equal(row.retries, 2);
}

// Pings: per-country daily counter.
{
  const env = makeEnv();
  await worker.fetch(req("/ping", { event: "fetch", version: "0.13.0", os: "macos" }, "PT"), env);
  assert.equal(env.store.get(`pingc:${today}:fetch:PT`), "1");
}

// Feedback: server-side scrub matches the shared cases.
{
  const env = makeEnv();
  for (const c of cases) {
    const r = await worker.fetch(req("/v1/feedback", { kind: "capability_request", task: c.in, tool: "wick_request_capability" }, "CH"), env);
    assert.equal(r.status, 204);
  }
  const tasks = feedbackRecords(env).map(r => r.task);
  for (const c of cases) {
    const got = tasks.find(t => c.contains.every(w => t.includes(w)));
    assert.ok(got, `no scrubbed record for ${JSON.stringify(c.in)}: ${JSON.stringify(tasks)}`);
    for (const bad of c.absent) assert.ok(!got.includes(bad), `${JSON.stringify(got)} keeps ${bad}`);
  }
  const rec = feedbackRecords(env)[0];
  assert.equal(rec.country, "CH");
  assert.equal(rec.tool, "wick_request_capability");
}

// Feedback: validation, per-caller limit, daily cap.
{
  const env = makeEnv();
  assert.equal((await worker.fetch(req("/v1/feedback", { task: "" }), env)).status, 400);

  const limited = makeEnv({ limiterAllows: false });
  assert.equal((await worker.fetch(req("/v1/feedback", { task: "x" }), limited)).status, 429);
  assert.equal(feedbackRecords(limited).length, 0);

  const full = makeEnv();
  full.store.set(`fbcount:${today}`, "100");
  assert.equal((await worker.fetch(req("/v1/feedback", { task: "x" }), full)).status, 204);
  assert.equal(feedbackRecords(full).length, 0);

  const failing = makeEnv();
  const put = failing.SUBSCRIPTIONS.put;
  failing.SUBSCRIPTIONS.put = async (k, v) => { if (k.startsWith("fbcount:")) throw new Error("KV write limit"); return put(k, v); };
  assert.equal((await worker.fetch(req("/v1/feedback", { task: "x" }), failing)).status, 204);
  assert.equal(feedbackRecords(failing).length, 0, "counter write failure must not store an uncounted record");
}

console.log("worker smoke tests passed");
