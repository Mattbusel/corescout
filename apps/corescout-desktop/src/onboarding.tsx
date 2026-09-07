/**
 * The first five minutes.
 *
 * # What this is not
 *
 * Not a tour, not a permissions wall, not five screens of value proposition.
 * Five short screens, each of which either says one thing or asks one
 * question, and the last of which starts the product.
 *
 * # Why the mode question is asked here
 *
 * Because asking it later means shipping a default that nobody chose. Asked
 * here, with three sentences, a person makes a real decision in about four
 * seconds, and the setting means something afterwards.
 */

import { useState } from "react";
import { call, type Setup } from "./api";
import { useCall } from "./components";

const DONE_KEY = "corescout.onboarded";

/** Whether the first run is over. */
export function isOnboarded(): boolean {
  try {
    return window.localStorage.getItem(DONE_KEY) === "yes";
  } catch {
    // Storage can be unavailable. Showing the welcome again is a small
    // annoyance; refusing to start the application is not.
    return false;
  }
}

function markOnboarded() {
  try {
    window.localStorage.setItem(DONE_KEY, "yes");
  } catch {
    /* nothing to do; the welcome will appear again, which is survivable */
  }
}

export function Onboarding({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState(0);
  const setups = useCall<Setup[]>("setup", {});
  const [mode, setMode] = useState("suggest");
  const [chosen, setChosen] = useState<Setup | null>(null);

  const next = () => setStep((n) => n + 1);
  const finish = async () => {
    await call("autonomy", { mode }).catch(() => undefined);
    markOnboarded();
    onDone();
  };

  const screens = [
    <div key="meet">
      <h1>Meet CoreScout</h1>
      <p className="lede">
        Your AI can learn a lot. Its computer usually learns nothing.
      </p>
      <p className="lede">CoreScout changes that.</p>
      <div className="row" style={{ marginTop: "var(--s6)" }}>
        <button className="action primary" onClick={next}>
          Continue
        </button>
      </div>
    </div>,

    <div key="watch">
      <h1>It watches how the two of them work together</h1>
      <p className="lede">
        Which commands your AI runs, which ones fail, what it tries next, and
        what the machine was doing at the time.
      </p>
      <p className="lede">
        Everything stays on this computer. Nothing is sent anywhere, there is no
        account, and secrets are stripped out before anything is written down.
      </p>
      <div className="row" style={{ marginTop: "var(--s6)" }}>
        <button className="action primary" onClick={next}>
          Continue
        </button>
      </div>
    </div>,

    <div key="connect">
      <h1>Connect your AI</h1>
      <p className="lede">
        Pick the one you use. You can add others later, and anything that
        speaks the Model Context Protocol will work.
      </p>
      <div className="modes" style={{ marginTop: "var(--s5)" }}>
        {setups.data?.map((setup) => (
          <button
            key={setup.agent}
            className="mode"
            aria-pressed={chosen?.agent === setup.agent}
            onClick={() => setChosen(setup)}
          >
            <span className="pip" />
            <span>
              <strong>{setup.title}</strong>
              <span>{setup.instructions[0]}</span>
            </span>
          </button>
        ))}
      </div>
      {chosen ? (
        <div className="card" style={{ marginTop: "var(--s4)" }}>
          <pre className="detail">{chosen.command ?? chosen.snippet}</pre>
          <div className="card-meta">
            <button
              className="action"
              onClick={() => {
                void navigator.clipboard?.writeText(chosen.command ?? chosen.snippet);
              }}
            >
              Copy
            </button>
            {chosen.automatic ? (
              <button
                className="action primary"
                onClick={() => {
                  void call("configure", { agent: chosen.agent }).catch(() => undefined);
                }}
              >
                Do it for me
              </button>
            ) : null}
          </div>
        </div>
      ) : null}
      <div className="row" style={{ marginTop: "var(--s6)" }}>
        <button className="action primary" onClick={next}>
          Continue
        </button>
        <button className="action" onClick={next}>
          Skip for now
        </button>
      </div>
    </div>,

    <div key="mode">
      <h1>Choose how much CoreScout can do</h1>
      <p className="lede">You can change this at any time, and pause everything instantly.</p>
      <div className="modes" style={{ marginTop: "var(--s5)" }}>
        {[
          {
            id: "observe",
            title: "Observe",
            summary: "CoreScout watches and learns. It changes nothing.",
          },
          {
            id: "suggest",
            title: "Suggest",
            summary: "CoreScout suggests improvements. You approve every change.",
          },
          {
            id: "assist",
            title: "Assist",
            summary:
              "CoreScout makes small reversible changes on its own. Anything bigger waits for you.",
          },
        ].map((option) => (
          <button
            key={option.id}
            className="mode"
            aria-pressed={mode === option.id}
            onClick={() => setMode(option.id)}
          >
            <span className="pip" />
            <span>
              <strong>{option.title}</strong>
              <span>{option.summary}</span>
            </span>
          </button>
        ))}
      </div>
      <p className="faint" style={{ marginTop: "var(--s4)" }}>
        There is a fourth, Autopilot, in Settings. It is worth having only once
        CoreScout has verified something on your machine.
      </p>
      <div className="row" style={{ marginTop: "var(--s5)" }}>
        <button className="action primary" onClick={next}>
          Continue
        </button>
      </div>
    </div>,

    <div key="start">
      <h1>Start learning</h1>
      <p className="lede">
        Use your AI exactly as you normally would. CoreScout will not interrupt
        you.
      </p>
      <p className="lede">
        Come back in a day. If it has found something, it will be on the first
        screen, with the evidence behind it.
      </p>
      <div className="row" style={{ marginTop: "var(--s6)" }}>
        <button className="action primary" onClick={() => void finish()}>
          Open CoreScout
        </button>
      </div>
    </div>,
  ];

  return (
    <div className="onboard">
      <div className="onboard-inner">
        <div className="steps" aria-hidden="true">
          {screens.map((_, index) => (
            <i key={index} className={index <= step ? "done" : ""} />
          ))}
        </div>
        {screens[step]}
        {step > 0 && step < screens.length - 1 ? (
          <button
            className="action"
            style={{ marginTop: "var(--s5)", border: "none", background: "transparent" }}
            onClick={() => setStep((n) => Math.max(0, n - 1))}
          >
            Back
          </button>
        ) : null}
      </div>
    </div>
  );
}
