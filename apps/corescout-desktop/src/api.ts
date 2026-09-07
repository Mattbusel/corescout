/**
 * Talking to CoreScout.
 *
 * The window never speaks HTTP. Every call goes through one Tauri command,
 * which holds the loopback token and forwards to the service. That keeps the
 * token out of the web view entirely: a page in this window has no way to
 * learn it and nothing to leak.
 *
 * Outside Tauri — `npm run dev` in a browser — the same functions resolve
 * against a small set of fixtures, so the interface can be worked on without a
 * service running. The fixtures are clearly marked and never reachable from
 * the shipped application.
 */

export type Json = unknown;

type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

interface TauriWindow {
  __TAURI_INTERNALS__?: unknown;
  __TAURI__?: { core?: { invoke?: Invoke } };
}

/** Whether this is running inside the desktop shell. */
export function inShell(): boolean {
  const w = window as unknown as TauriWindow;
  return Boolean(w.__TAURI_INTERNALS__ ?? w.__TAURI__);
}

let invoker: Invoke | null = null;

async function getInvoke(): Promise<Invoke> {
  if (invoker) return invoker;
  const mod = await import("@tauri-apps/api/core");
  invoker = mod.invoke as unknown as Invoke;
  return invoker;
}

/** An error carrying the service's own words, which are written to be read. */
export class CoreScoutError extends Error {}

/** Call a CoreScout method. */
export async function call(method: string, params: Record<string, unknown> = {}): Promise<Json> {
  if (!inShell()) return fixture(method);
  try {
    const invoke = await getInvoke();
    return await invoke("corescout_call", { method, params });
  } catch (error) {
    throw new CoreScoutError(String(error));
  }
}

/** Ask the shell to do something only it can. */
export async function shell(command: string, args: Record<string, unknown> = {}): Promise<Json> {
  if (!inShell()) return null;
  const invoke = await getInvoke();
  return invoke(command, args);
}

/** Whether CoreScout starts at sign-in, and who decides. */
export interface Startup {
  enabled: boolean;
  /** False inside a Store install, where Windows owns this. */
  changeable: boolean;
  explain: string;
}

/** Whether CoreScout starts when this user signs in. */
export async function launchAtLogin(): Promise<Startup | null> {
  return (await shell("launch_at_login")) as Startup | null;
}

/** Turn starting at sign-in on or off, and report what it ended up as. */
export async function setLaunchAtLogin(enabled: boolean): Promise<Startup | null> {
  return (await shell("set_launch_at_login", { enabled })) as Startup | null;
}

/** Open a link in the user's own browser, or the Store. */
export async function openExternal(url: string): Promise<void> {
  await shell("open_external", { url });
}


/** Open the folder CoreScout keeps its data in. */
export async function revealDataFolder(): Promise<void> {
  await shell("reveal_data_folder");
}

/* --------------------------------------------------------------- shapes */

export interface Status {
  running: boolean;
  version: string;
  uptime_ms: number;
  learning_for_ms: number;
  autonomy: string;
  paused: boolean;
  observing: boolean;
  connected: string[];
  observations: number;
  states: number;
  actions: number;
}

export interface Card {
  id: string;
  title: string;
  detail: string;
  basis: string;
  causal: boolean;
  confidence: number;
  reliability?: number;
  uses: number;
  last_ms: number;
  usable_by_ai: boolean;
  is_capability: boolean;
  approved: boolean;
}

export interface Mood {
  plain: string;
  state?: number;
  seen: number;
  confidence?: number;
  unfamiliar: boolean;
}

export interface Home {
  headline: string;
  connected: Agent[];
  today: { learned: number; capabilities: number; retired: number; rejected: number };
  machine: Mood;
  latest?: Card;
  empty?: { title: string; body: string };
}

export interface Agent {
  id: string;
  name: string;
  connected: boolean;
  summary: string;
  actions: number;
  sessions: number;
  capabilities: number;
  last_seen_ms: number;
}

export interface Moment {
  sequence: number;
  at_ms: number;
  summary: string;
  kind: string;
  severity: string;
  subject?: string;
  reason?: string;
  actual?: string;
  detail?: unknown;
}

export interface Node {
  key: string;
  label: string;
  class: string;
  activity: number;
  clarity: number;
  parent?: string;
}

export interface Live {
  plain: string;
  state?: number;
  seen: number;
  nodes: Node[];
  trail: number[];
  recent_states: number[];
  agent_activity: string[];
}

export interface Explanation {
  id: string;
  title: string;
  simple: string;
  evidence: string;
  kind: string;
  confidence: number;
  alternative?: string;
  result?: string;
  technical?: unknown;
}

export interface Setup {
  agent: string;
  title: string;
  automatic: boolean;
  config_path?: string;
  snippet: string;
  command?: string;
  instructions: string[];
  /** The hooks block, where CoreScout can watch this client directly. */
  hook_snippet?: string;
  hook_path?: string;
}

export interface Question {
  id: string;
  name: string;
  about: string;
  settled: boolean;
  missing: string;
  randomised_trials: number;
  observed_trials: number;
}

export interface Knowledge {
  learning_for_ms: number;
  machine_states: number;
  workflow_patterns: number;
  verified_capabilities: number;
  open_questions: number;
  rejected: number;
  retired: number;
  about_agents: string[];
  about_workspaces: string[];
  about_machine: string[];
  still_uncertain: string[];
}

export interface Privacy {
  data_dir: string;
  bytes: number;
  telemetry: boolean;
  holdings: { kind: string; describes: string; count: number; agent_activity: boolean }[];
}

export interface Diagnostics {
  mean_observe_ns: number;
  worst_observe_ns: number;
  observations: number;
  interval_ms: number;
  cpu_share: number;
  ring_bytes: number;
  ring_samples: number;
  ring_wrapped: boolean;
  documents: number;
  events: number;
  schema: number;
  gaps: string[];
}

export interface Machine {
  cpu: string;
  cores: number;
  threads: number;
  hybrid: boolean;
  entities: number;
  channels: number;
  observed_cells: number;
  observe_ns: number;
  self_description: string[];
}

/**
 * What the Microsoft Store says about this copy.
 *
 * `null` when this build was not installed from the Store, which is every
 * development build. Nothing in the interface behaves differently: the Store
 * enforces the licence by not letting an unlicensed copy run.
 */
export type Licence = {
  active: boolean;
  trial: boolean;
  trial_days_left: number | null;
} | null;

/** What this installation has worked out, and one line about the licence. */
export interface WorkedOut {
  headline: string;
  trial: boolean;
  trial_days_left: number | null;
  worth_mentioning: boolean;
  evidence: { value: string; label: string }[];
}

export interface Settings {
  autonomy: string;
  autonomy_title: string;
  autonomy_summary: string;
  paused: boolean;
  authority: string;
  folders: string[];
  commands: string[];
  limits: { per_hour: number; per_day: number; failure_streak: number };
  modes: { id: string; title: string; summary: string }[];
}

/* -------------------------------------------------------------- fixtures */

/**
 * What the browser sees during `npm run dev`.
 *
 * Deliberately sparse: a fixture that shows a full, impressive dashboard makes
 * it easy to design a screen that only looks right when there is data, which
 * is the state nobody is in on their first day.
 */
function fixture(method: string): Json {
  switch (method) {
    case "status":
      return {
        running: true,
        version: "dev",
        uptime_ms: 0,
        learning_for_ms: 0,
        autonomy: "suggest",
        paused: false,
        observing: false,
        connected: [],
        observations: 0,
        states: 0,
        actions: 0,
      } satisfies Status;
    case "home":
      return {
        headline: "Your computer is watching how it runs.",
        connected: [],
        today: { learned: 0, capabilities: 0, retired: 0, rejected: 0 },
        machine: {
          plain: "Not connected to a running CoreScout service.",
          seen: 0,
          unfamiliar: false,
        },
        empty: {
          title: "This is the browser preview.",
          body: "Run the desktop application to see real data. Nothing here is real.",
        },
      } satisfies Home;
    case "learned":
    case "capabilities":
    case "agents":
    case "activity":
    case "states":
    case "failures":
    case "hypotheses":
      return [];
    case "setup":
      return [];
    case "licence":
      return null satisfies Licence;
    case "worked_out":
      return {
        headline: "CoreScout is not running.",
        trial: false,
        trial_days_left: null,
        worth_mentioning: false,
        evidence: [],
      } satisfies WorkedOut;
    case "live":
      return { plain: "No service.", seen: 0, nodes: [], trail: [], recent_states: [], agent_activity: [] };
    default:
      return {};
  }
}
