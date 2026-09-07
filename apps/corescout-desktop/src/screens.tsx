/**
 * The six screens.
 *
 * # The rule every one of them follows
 *
 * The first thing on the screen is a sentence. Numbers come after it, and the
 * technical layer comes after those, behind a control. A screen that opens
 * with a table has already failed the person who installed this because their
 * AI kept failing at something.
 */

import { useState } from "react";
import {
  call,
  type Agent,
  type Card,
  type Diagnostics,
  type Explanation,
  type Home as HomeData,
  type Knowledge,
  type Live,
  type Machine,
  type Moment,
  type Privacy,
  type Question,
  type Setup,
  type Settings as SettingsData,
  type Status,
} from "./api";
import {
  Badge,
  Confidence,
  Depth,
  Do,
  Empty,
  EvidenceBadge,
  Loading,
  Problem,
  Sheet,
  Stat,
  useCall,
} from "./components";
import { ago, bytes, clock, count, cpuShare, duration, nanos, share } from "./format";
import { MirrorView, Sparkline } from "./mirror";

/* ------------------------------------------------------------------ home */

export function Home({ onGo }: { onGo: (screen: string) => void }) {
  const home = useCall<HomeData>("home", {}, 4000);
  const live = useCall<Live>("live", {}, 1200);
  const [open, setOpen] = useState<string | null>(null);

  if (home.error) return <Problem message={home.error} retry={home.reload} />;
  if (!home.data) return <Loading lines={4} />;
  const data = home.data;

  return (
    <div className="page">
      <h1>{data.headline}</h1>
      <p className="lede">
        {data.connected.some((agent) => agent.connected)
          ? "Everything it learns stays on this computer."
          : "Connect an AI and it will start learning how the two of them work together."}
      </p>

      <div className="section">
        {live.data ? (
          <MirrorView
            nodes={live.data.nodes}
            stateId={live.data.state}
            seen={live.data.seen}
            plain={data.machine.plain}
            unfamiliar={data.machine.unfamiliar}
          />
        ) : (
          <div className="skeleton" style={{ height: 320 }} />
        )}
      </div>

      <div className="section stat-row">
        <Stat value={data.today.learned} label="learned today" />
        <Stat value={data.today.capabilities} label="new capabilities" />
        <Stat value={data.today.retired} label="beliefs withdrawn" />
        <Stat
          value={data.connected.filter((agent) => agent.connected).length}
          label="AI connected"
        />
      </div>

      {data.empty ? (
        <div className="section">
          <Empty title={data.empty.title} body={data.empty.body} />
          <div className="row" style={{ marginTop: "var(--s4)", justifyContent: "center" }}>
            <button className="action primary" onClick={() => onGo("ai")}>
              Connect your AI
            </button>
          </div>
        </div>
      ) : null}

      {data.latest ? (
        <div className="section">
          <div className="eyebrow">Latest</div>
          <LearnedCard card={data.latest} onOpen={setOpen} />
        </div>
      ) : null}

      {open ? <ExplainSheet id={open} onClose={() => setOpen(null)} /> : null}
    </div>
  );
}

/* --------------------------------------------------------------- learned */

export function Learned() {
  const cards = useCall<Card[]>("learned", {}, 6000);
  const open_ = useCall<Question[]>("hypotheses", {}, 10_000);
  const [open, setOpen] = useState<string | null>(null);

  if (cards.error) return <Problem message={cards.error} retry={cards.reload} />;
  if (!cards.data) return <Loading lines={5} />;

  const verified = cards.data.filter((card) => card.causal);
  const noticed = cards.data.filter((card) => !card.causal);

  return (
    <div className="page">
      <h1>What your computer has learned</h1>
      <p className="lede">
        CoreScout tells you how it knows each of these. Something it has only
        seen happen together is not the same as something it has tested.
      </p>

      {cards.data.length === 0 ? (
        <div className="section">
          <Empty
            title="Nothing yet."
            body="That's normal. Use your AI as usual. CoreScout will start noticing what repeats, and it only promotes a discovery once it survives its evidence requirements."
          />
        </div>
      ) : null}

      {verified.length > 0 ? (
        <div className="section">
          <div className="eyebrow">Verified</div>
          {verified.map((card) => (
            <LearnedCard key={card.id} card={card} onOpen={setOpen} onChange={cards.reload} />
          ))}
        </div>
      ) : null}

      {noticed.length > 0 ? (
        <div className="section">
          <div className="eyebrow">Noticed, not yet tested</div>
          {noticed.map((card) => (
            <LearnedCard key={card.id} card={card} onOpen={setOpen} onChange={cards.reload} />
          ))}
        </div>
      ) : null}

      {open_.data && open_.data.length > 0 ? (
        <div className="section">
          <div className="eyebrow">Still uncertain</div>
          <p className="muted">
            CoreScout is testing these. Until it has enough randomised trials it
            will not claim one way or the other.
          </p>
          {open_.data.map((question) => (
            <div className="card" key={question.id}>
              <h3>{question.name}</h3>
              <p>{question.missing}</p>
              <div className="card-meta">
                <span>
                  {question.randomised_trials} randomised of {question.observed_trials} trials
                </span>
              </div>
            </div>
          ))}
        </div>
      ) : null}

      {open ? <ExplainSheet id={open} onClose={() => setOpen(null)} /> : null}
    </div>
  );
}

function LearnedCard({
  card,
  onOpen,
  onChange,
}: {
  card: Card;
  onOpen: (id: string) => void;
  onChange?: () => void;
}) {
  return (
    <div className="card interactive" onClick={() => onOpen(card.id)}>
      <div className="card-head">
        <h3>{card.title}</h3>
        <EvidenceBadge causal={card.causal} />
      </div>
      <p>{card.detail}</p>
      <div className="card-meta">
        <Confidence value={card.confidence} causal={card.causal} />
        {card.is_capability ? (
          <span>
            {card.uses === 0
              ? "never used"
              : `${share(card.reliability, "never used")} of ${count(card.uses, "use")} worked`}
          </span>
        ) : null}
        {card.last_ms ? <span className="faint">{ago(card.last_ms)}</span> : null}
        {card.usable_by_ai ? <Badge kind="quiet">your AI can use this</Badge> : null}
      </div>
      {card.is_capability && onChange ? (
        <div className="card-meta" onClick={(event) => event.stopPropagation()}>
          {card.approved ? (
            <>
              <Do
                method="decide"
                params={{ id: card.id, approved: true, auto_use: !card.usable_by_ai }}
                label={card.usable_by_ai ? "Ask me each time" : "Let my AI use it"}
                onDone={onChange}
              />
              <Do
                method="decide"
                params={{ id: card.id, approved: true, disabled: true }}
                label="Switch off"
                onDone={onChange}
              />
            </>
          ) : (
            <Do
              method="decide"
              params={{ id: card.id, approved: true }}
              label="Approve"
              primary
              onDone={onChange}
            />
          )}
          <Do
            method="run"
            params={{ id: card.id, dry_run: true }}
            label="Test run"
            onDone={onChange}
          />
        </div>
      ) : null}
    </div>
  );
}

function ExplainSheet({ id, onClose }: { id: string; onClose: () => void }) {
  const explanation = useCall<Explanation>("explain", { id });
  const [technical, setTechnical] = useState(false);

  return (
    <Sheet onClose={onClose}>
      <div className="spread" style={{ marginBottom: "var(--s5)" }}>
        <div className="eyebrow" style={{ margin: 0 }}>
          Why CoreScout believes this
        </div>
        <button className="action" onClick={onClose}>
          Close
        </button>
      </div>
      {explanation.error ? <Problem message={explanation.error} /> : null}
      {!explanation.data ? (
        <Loading />
      ) : (
        <>
          <h1>{explanation.data.title}</h1>
          <p className="lede">{explanation.data.simple}</p>

          <div className="section">
            <div className="eyebrow">The evidence</div>
            <div className="card">
              <div className="card-head">
                <h3>
                  {explanation.data.kind === "randomised"
                    ? "Measured under randomised assignment"
                    : "Observed, not tested"}
                </h3>
                <EvidenceBadge causal={explanation.data.kind === "randomised"} />
              </div>
              <p>{explanation.data.evidence}</p>
            </div>
          </div>

          <div className="section">
            <dl className="kv">
              <dt>Confidence</dt>
              <dd>
                <Confidence
                  value={explanation.data.confidence}
                  causal={explanation.data.kind === "randomised"}
                />
              </dd>
              {explanation.data.alternative ? (
                <>
                  <dt>Alternative</dt>
                  <dd>{explanation.data.alternative}</dd>
                </>
              ) : null}
              {explanation.data.result ? (
                <>
                  <dt>Result</dt>
                  <dd>{explanation.data.result}</dd>
                </>
              ) : null}
            </dl>
          </div>

          <div className="section">
            <div className="spread" style={{ marginBottom: "var(--s3)" }}>
              <div className="eyebrow" style={{ margin: 0 }}>
                Underneath
              </div>
              <Depth technical={technical} onChange={setTechnical} />
            </div>
            {technical ? (
              <pre className="detail">
                {JSON.stringify(explanation.data.technical ?? {}, null, 2)}
              </pre>
            ) : (
              <p className="muted">
                Everything above is derived from records CoreScout keeps on this
                machine. Switch to Technical to read them.
              </p>
            )}
          </div>
        </>
      )}
    </Sheet>
  );
}

/* ------------------------------------------------------------------- ai */

export function AI() {
  const agents = useCall<Agent[]>("agents", {}, 5000);
  const setups = useCall<Setup[]>("setup", {});
  const [chosen, setChosen] = useState<string | null>(null);

  return (
    <div className="page">
      <h1>Your AI</h1>
      <p className="lede">
        CoreScout works with anything that speaks the Model Context Protocol.
        Connecting one takes a single line.
      </p>

      <div className="section">
        <div className="eyebrow">Connected</div>
        {agents.data && agents.data.length > 0 ? (
          agents.data.map((agent) => (
            <div className="card" key={agent.id}>
              <div className="card-head">
                <h3>{agent.name}</h3>
                <Badge kind={agent.connected ? "verified" : "quiet"}>
                  {agent.connected ? "Connected" : "Not connected"}
                </Badge>
              </div>
              <p>{agent.summary}</p>
              <div className="card-meta">
                <span>{count(agent.actions, "action")} observed</span>
                <span>{count(agent.sessions, "session")}</span>
                <span>{count(agent.capabilities, "capability", "capabilities")} available</span>
              </div>
            </div>
          ))
        ) : (
          <Empty
            title="No AI has connected yet."
            body="Pick one below. CoreScout generates the exact configuration and can usually write it for you."
          />
        )}
      </div>

      <div className="section">
        <div className="eyebrow">Connect one</div>
        {setups.data?.map((setup) => (
          <div className="card" key={setup.agent}>
            <div className="card-head">
              <h3>{setup.title}</h3>
              {setup.automatic ? <Badge kind="quiet">one click</Badge> : null}
            </div>
            <ul className="list" style={{ marginTop: "var(--s2)" }}>
              {setup.instructions.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
            <div className="card-meta">
              <button
                className="action"
                onClick={() => setChosen(chosen === setup.agent ? null : setup.agent)}
              >
                {chosen === setup.agent ? "Hide" : "Show configuration"}
              </button>
              <button
                className="action"
                onClick={() => {
                  void navigator.clipboard?.writeText(setup.command ?? setup.snippet);
                }}
              >
                Copy
              </button>
              {setup.automatic ? (
                <Do
                  method="configure"
                  params={{ agent: setup.agent }}
                  label="Configure automatically"
                  primary
                  onDone={agents.reload}
                />
              ) : null}
            </div>
            {chosen === setup.agent ? (
              <>
                {setup.config_path ? (
                  <p className="faint" style={{ marginTop: "var(--s3)" }}>
                    {setup.config_path}
                  </p>
                ) : null}
                <pre className="detail">{setup.command ?? setup.snippet}</pre>
              </>
            ) : null}
          </div>
        ))}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------- activity */

export function Activity() {
  const [technical, setTechnical] = useState(false);
  const moments = useCall<Moment[]>("activity", { limit: 120, technical }, 3000);

  return (
    <div className="page">
      <div className="spread">
        <h1>Activity</h1>
        <Depth technical={technical} onChange={setTechnical} />
      </div>
      <p className="lede">Everything CoreScout has done, and why.</p>

      <div className="section">
        {moments.error ? <Problem message={moments.error} retry={moments.reload} /> : null}
        {moments.data && moments.data.length === 0 ? (
          <Empty
            title="Nothing has happened yet."
            body="CoreScout writes down what it notices, what it suggests, and every change it makes. This fills up as it works."
          />
        ) : null}
        <div className="timeline">
          {moments.data?.map((moment) => (
            <div
              key={moment.sequence}
              className={`moment${
                moment.severity === "error" || moment.severity === "warning"
                  ? " bad"
                  : moment.kind === "discovery" || moment.kind === "action"
                    ? " notable"
                    : ""
              }`}
            >
              <time>{clock(moment.at_ms)}</time>
              <div>{moment.summary}</div>
              {moment.reason ? <div className="why">Why: {moment.reason}</div> : null}
              {moment.actual ? <div className="why">Result: {moment.actual}</div> : null}
              {technical && moment.detail ? (
                <pre className="detail">{JSON.stringify(moment.detail, null, 2)}</pre>
              ) : null}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------- computer */

export function Computer() {
  const machine = useCall<Machine>("computer", {}, 5000);
  const live = useCall<Live>("live", {}, 1500);
  const knowledge = useCall<Knowledge>("knowledge", {}, 10_000);
  const states = useCall<{ id: number; plain: string; entries: number; mean_dwell_ms: number }[]>(
    "states",
    {},
    10_000,
  );
  const [technical, setTechnical] = useState(false);

  return (
    <div className="page wide">
      <div className="spread">
        <h1>This computer</h1>
        <Depth technical={technical} onChange={setTechnical} />
      </div>

      {live.data ? (
        <div className="section">
          <MirrorView
            nodes={live.data.nodes}
            stateId={live.data.state}
            seen={live.data.seen}
            plain={live.data.plain}
            unfamiliar={false}
          />
          {live.data.trail.length > 2 ? <Sparkline values={live.data.trail} /> : null}
        </div>
      ) : null}

      {machine.data ? (
        <div className="section">
          <h2>{machine.data.cpu}</h2>
          <p className="muted">
            {count(machine.data.cores, "core")}, {count(machine.data.threads, "thread")}
            {machine.data.hybrid ? ", of more than one kind" : ""}.
          </p>
          {technical ? (
            <dl className="kv">
              <dt>Entities</dt>
              <dd>{machine.data.entities}</dd>
              <dt>Channels each</dt>
              <dd>{machine.data.channels}</dd>
              <dt>Readable now</dt>
              <dd>
                {machine.data.observed_cells} of{" "}
                {machine.data.entities * machine.data.channels}
              </dd>
              <dt>Observation cost</dt>
              <dd>{nanos(machine.data.observe_ns)}</dd>
            </dl>
          ) : null}
        </div>
      ) : null}

      {knowledge.data ? (
        <div className="section">
          <div className="eyebrow">What it knows</div>
          <p className="lede">
            Your computer has been learning for {duration(knowledge.data.learning_for_ms)}.
          </p>
          <div className="stat-row" style={{ marginTop: "var(--s4)" }}>
            <Stat value={knowledge.data.machine_states} label="recurring states" />
            <Stat value={knowledge.data.workflow_patterns} label="workflow patterns" />
            <Stat value={knowledge.data.verified_capabilities} label="verified procedures" />
            <Stat value={knowledge.data.open_questions} label="open questions" />
            <Stat value={knowledge.data.rejected} label="ideas rejected" />
          </div>

          <Knows title="About your AI" lines={knowledge.data.about_agents} />
          <Knows title="About the work" lines={knowledge.data.about_workspaces} />
          <Knows title="About this machine" lines={knowledge.data.about_machine} />
          <Knows
            title="Still uncertain"
            lines={knowledge.data.still_uncertain}
            empty="Nothing open right now."
          />
        </div>
      ) : null}

      {technical && states.data && states.data.length > 0 ? (
        <div className="section">
          <div className="eyebrow">States it found in itself</div>
          {states.data.map((state) => (
            <div className="card" key={state.id}>
              <div className="card-head">
                <h3>State {state.id}</h3>
                <span className="mono faint">{state.entries} entries</span>
              </div>
              <p>{state.plain}</p>
            </div>
          ))}
          <p className="faint" style={{ marginTop: "var(--s3)" }}>
            Those numbers are the machine's own. Nothing told it how many kinds
            of state it has.
          </p>
        </div>
      ) : null}
    </div>
  );
}

function Knows({
  title,
  lines,
  empty,
}: {
  title: string;
  lines: string[];
  empty?: string;
}) {
  if (lines.length === 0 && !empty) return null;
  return (
    <div style={{ marginTop: "var(--s5)" }}>
      <h2>{title}</h2>
      {lines.length === 0 ? (
        <p className="muted">{empty}</p>
      ) : (
        <ul className="list">
          {lines.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      )}
    </div>
  );
}

/* ------------------------------------------------------------- settings */

export function Settings({ status }: { status: Status | null }) {
  const settings = useCall<SettingsData>("settings", {}, 4000);
  const privacy = useCall<Privacy>("privacy", {}, 15_000);
  const diagnostics = useCall<Diagnostics>("diagnostics", {}, 5000);
  const [tab, setTab] = useState<"general" | "privacy" | "diagnostics">("general");

  return (
    <div className="page">
      <h1>Settings</h1>

      <div className="row" style={{ marginBottom: "var(--s5)" }}>
        <div className="toolbar">
          {(["general", "privacy", "diagnostics"] as const).map((name) => (
            <button key={name} aria-pressed={tab === name} onClick={() => setTab(name)}>
              {name === "general" ? "General" : name === "privacy" ? "Privacy" : "Diagnostics"}
            </button>
          ))}
        </div>
      </div>

      {tab === "general" && settings.data ? (
        <>
          {settings.data.paused ? (
            <div className="banner stop">
              <span style={{ flex: 1 }}>
                CoreScout is paused. It is still watching and learning; it will
                not change anything.
              </span>
              <Do method="resume" label="Resume" onDone={settings.reload} />
            </div>
          ) : null}

          <div className="section">
            <h2>How much CoreScout can do</h2>
            <div className="modes">
              {settings.data.modes.map((mode) => (
                <button
                  key={mode.id}
                  className="mode"
                  aria-pressed={settings.data?.autonomy === mode.id}
                  onClick={async () => {
                    await call("autonomy", { mode: mode.id });
                    settings.reload();
                  }}
                >
                  <span className="pip" />
                  <span>
                    <strong>{mode.title}</strong>
                    <span>{mode.summary}</span>
                  </span>
                </button>
              ))}
            </div>
            {!settings.data.paused ? (
              <div className="row" style={{ marginTop: "var(--s4)" }}>
                <Do method="pause" label="Pause CoreScout" danger onDone={settings.reload} />
                <span className="faint">Stops every change immediately.</span>
              </div>
            ) : null}
          </div>

          <div className="section">
            <h2>What it may touch</h2>
            <p className="muted">{settings.data.authority}</p>
            {settings.data.folders.length > 0 ? (
              <ul className="list">
                {settings.data.folders.map((folder) => (
                  <li key={folder} className="mono">
                    {folder}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="faint">
                No folders shared. CoreScout can only act on itself until you
                share one.
              </p>
            )}
            <p className="faint" style={{ marginTop: "var(--s3)" }}>
              At most {settings.data.limits.per_hour} changes an hour and{" "}
              {settings.data.limits.per_day} a day. It stops after{" "}
              {settings.data.limits.failure_streak} failures in a row and waits
              for you.
            </p>
          </div>
        </>
      ) : null}

      {tab === "privacy" && privacy.data ? (
        <>
          <div className="section">
            <h2>Everything stays here</h2>
            <p className="lede">
              CoreScout sends nothing anywhere. There is no account, no server
              and no telemetry. Everything it has is in one folder.
            </p>
            <div className="card">
              <div className="mono">{privacy.data.data_dir}</div>
              <div className="card-meta">
                <span>{bytes(privacy.data.bytes)} on disk</span>
                <Badge kind={privacy.data.telemetry ? "warn" : "verified"}>
                  {privacy.data.telemetry ? "telemetry on" : "no telemetry"}
                </Badge>
              </div>
            </div>
          </div>

          <div className="section">
            <h2>What it holds</h2>
            {privacy.data.holdings
              .filter((holding) => holding.count > 0)
              .map((holding) => (
                <div className="card" key={holding.kind}>
                  <div className="card-head">
                    <h3>{holding.describes}</h3>
                    <span className="mono faint">{holding.count}</span>
                  </div>
                  {holding.agent_activity ? (
                    <p className="faint">Removed when you forget your AI history.</p>
                  ) : null}
                </div>
              ))}
            <p className="faint" style={{ marginTop: "var(--s3)" }}>
              Commands are stored with secrets stripped before anything is
              written down. Source code is never read or stored.
            </p>
          </div>

          <div className="section">
            <h2>Delete</h2>
            <div className="row">
              <Do
                method="forget"
                params={{ everything: false }}
                label="Forget my AI history"
                confirm="Sessions, commands and outcomes go. What CoreScout learned about this machine stays."
                onDone={privacy.reload}
              />
              <Do
                method="forget"
                params={{ everything: true }}
                label="Forget everything"
                danger
                confirm="Every record goes, including the states this computer found in itself. This cannot be undone."
                onDone={privacy.reload}
              />
            </div>
          </div>
        </>
      ) : null}

      {tab === "diagnostics" && diagnostics.data ? (
        <div className="section">
          <h2>What CoreScout costs</h2>
          <p className="lede">Measured on this machine, not quoted from anywhere.</p>
          <div className="stat-row" style={{ marginTop: "var(--s4)" }}>
            <Stat value={nanos(diagnostics.data.mean_observe_ns)} label="per observation" />
            <Stat value={cpuShare(diagnostics.data.cpu_share)} label="while running" />
            <Stat value={bytes(diagnostics.data.ring_bytes)} label="fixed history size" />
          </div>
          <dl className="kv" style={{ marginTop: "var(--s5)" }}>
            <dt>Worst pass</dt>
            <dd>{nanos(diagnostics.data.worst_observe_ns)}</dd>
            <dt>Interval</dt>
            <dd>{diagnostics.data.interval_ms} ms</dd>
            <dt>Observations</dt>
            <dd>{diagnostics.data.observations} this run</dd>
            <dt>History</dt>
            <dd>
              {diagnostics.data.ring_samples} samples
              {diagnostics.data.ring_wrapped ? ", oldest overwritten" : ""}
            </dd>
            <dt>Records</dt>
            <dd>
              {diagnostics.data.documents} documents, {diagnostics.data.events} events
            </dd>
            <dt>Storage schema</dt>
            <dd>version {diagnostics.data.schema}</dd>
            <dt>Version</dt>
            <dd>{status?.version ?? "unknown"}</dd>
          </dl>
          {diagnostics.data.gaps.length > 0 ? (
            <div style={{ marginTop: "var(--s5)" }}>
              <h2>What it cannot see</h2>
              <ul className="list">
                {diagnostics.data.gaps.map((gap) => (
                  <li key={gap}>{gap}</li>
                ))}
              </ul>
              <p className="faint">
                CoreScout records a gap rather than guessing at a value.
              </p>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
