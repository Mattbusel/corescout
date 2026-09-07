/**
 * Give a fresh CoreScout something real to have learned.
 *
 * # What is real and what is staged
 *
 * The *history* is staged: a week of AI work that did not happen, described
 * here. Everything CoreScout then says about it is real, computed by the
 * shipping engine under the same evidence rules a customer's installation
 * runs, including its refusal to call any of it more than a correlation.
 *
 * That distinction is the whole point of taking screenshots this way. A shot
 * showing "94% reliable" because somebody typed 94 into a design file is a
 * lie. A shot showing what the product concluded from a stated history is a
 * demonstration, and `LISTING.md` says which it is.
 *
 * The machine details -- processor, cores, entities, recurring states -- are
 * not staged at all. Those come from the computer this runs on.
 *
 * # Why this posts to the API rather than through the hook
 *
 * A hook takes its timing from the clock, because that is when the tool call
 * happened. That is right in production and useless here: a week of history
 * cannot be replayed in real time, and squeezed into one second every command
 * appears to precede every other one. The API accepts an explicit timestamp,
 * so the history can be laid out over days. It is the same engine, the same
 * ingestion, the same evidence rules.
 */

import { readFile } from "node:fs/promises";

const [endpointPath] = process.argv.slice(2);
if (!endpointPath) {
  console.error("usage: node seed.mjs <endpoint.json>");
  process.exit(2);
}
const endpoint = JSON.parse(await readFile(endpointPath, "utf8"));
const base = `http://${endpoint.host}:${endpoint.port}`;

async function call(method, params = {}) {
  const response = await fetch(`${base}/call`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${endpoint.token}`,
      "content-type": "application/json",
    },
    body: JSON.stringify({ method, params }),
  });
  const body = await response.json();
  if (!response.ok) throw new Error(body?.error ?? `CoreScout answered ${response.status}`);
  return body;
}

const WORKSPACE = "C:\\Projects\\payments-api";
const DAY = 24 * 60 * 60 * 1000;
const MINUTE = 60 * 1000;

// Nine days of history that ends a few minutes ago. A history that stops a
// week back leaves every "today" count at zero, which is accurate for that
// history and misleading as a picture of the product.
let clock = Date.now() - 9 * DAY;
let observed = 0;

/** One command, at a stated time, with a stated outcome. */
async function did(session, name, ok, { checked = true, gap = 20 } = {}) {
  clock += gap * MINUTE;
  observed += 1;
  await call("observe", {
    session,
    name,
    workspace: WORKSPACE,
    started_ms: clock,
    duration_ms: ok ? 9_000 + (observed % 7) * 800 : 4_000 + (observed % 5) * 600,
    reported: ok ? "success" : "failure",
    detail: ok ? "" : "error: schema.graphql is newer than the generated client",
    // A hook could not say this. An agent that actually checked can, and the
    // difference is the thing CoreScout is built to notice.
    verified: checked ? (ok ? "confirmed" : "contradicted") : undefined,
    verification_detail: ok ? "" : "the generated client did not match the schema",
  });
}

async function main() {
  await call("hello", { name: "claude-code", version: "2.1" });

  // Two ways of working over about a week. The careful sessions regenerate the
  // client before building and their builds pass; the hurried ones do not.
  // Nothing here tells CoreScout that; it has to notice, and it has to decline
  // to call it a cause.
  for (let round = 0; round < 20; round += 1) {
    // Twenty-five minutes after the previous round's test run, which is
    // further back than the ten minutes CoreScout looks over for a precursor.
    // At six minutes the test run also lands inside the window and CoreScout
    // correctly reports two correlations, which is true and reads as muddle.
    await did("careful", "npm run codegen", true, { gap: 25 });
    await did("careful", "npm run build", round % 10 !== 0, { gap: 3 });
    await did("careful", "npm test", true, { gap: 40 });
  }
  for (let round = 0; round < 16; round += 1) {
    await did("hurried", "npm run build", round % 6 === 0, { gap: 55 });
  }

  // A deploy nobody ever verifies. Not a failure -- an absence of evidence,
  // which CoreScout names rather than counts.
  for (let round = 0; round < 9; round += 1) {
    await did("careful", "npm run deploy:staging", true, { checked: false, gap: 90 });
  }

  // The last day, so the counts that are scoped to today are not all zero.
  clock = Date.now() - 6 * 60 * MINUTE;
  for (let round = 0; round < 4; round += 1) {
    await did("today", "npm run codegen", true, { gap: 25 });
    await did("today", "npm run build", true, { gap: 3 });
    await did("today", "npm test", true, { gap: 12 });
  }

  // Ordinary work, so the timeline is not one command repeated forty times.
  for (const command of [
    "git status",
    "git commit -m regenerated-the-client",
    "npm run lint",
    "git push",
    "npx prisma migrate dev",
    "git pull --rebase",
  ]) {
    await did("today", command, true, { gap: 4 });
  }

  console.log(`  ${observed} actions, laid out over nine days`);
  const learned = await call("learned");
  const failures = await call("failures");
  console.log(`  CoreScout concluded: ${learned.length} learned, ${failures.length} failure mode(s)`);
  for (const card of learned) {
    console.log(`    [${card.basis}] ${card.title}`);
  }
}

await main();
