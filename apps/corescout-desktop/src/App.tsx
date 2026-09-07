/**
 * The shell: six places to be, and the one status line that is always true.
 *
 * # Six, not fifteen
 *
 * Every screen here answers a question someone actually has. There is no
 * screen for a subsystem, and nothing is named after an internal component:
 * the words Mirror, Concept and Atlas appear nowhere a user can see, though
 * that is exactly what is underneath.
 */

import { useEffect, useState } from "react";
import { type Status } from "./api";
import { useCall } from "./components";
import { Activity, AI, Computer, Home, Learned, Settings } from "./screens";
import { isOnboarded, Onboarding } from "./onboarding";

const SCREENS = [
  { id: "home", label: "Home" },
  { id: "ai", label: "AI" },
  { id: "learned", label: "Learned" },
  { id: "activity", label: "Activity" },
  { id: "computer", label: "Computer" },
  { id: "settings", label: "Settings" },
] as const;

type ScreenId = (typeof SCREENS)[number]["id"];

/** Follow the operating system unless the user has said otherwise. */
function useTheme(): [string, (next: string) => void] {
  const [theme, setTheme] = useState(() => {
    try {
      return window.localStorage.getItem("corescout.theme") ?? "system";
    } catch {
      return "system";
    }
  });

  useEffect(() => {
    const apply = () => {
      const dark =
        theme === "dark" ||
        (theme === "system" &&
          window.matchMedia("(prefers-color-scheme: dark)").matches);
      document.documentElement.dataset.theme = dark ? "dark" : "light";
    };
    apply();
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);

  return [
    theme,
    (next: string) => {
      setTheme(next);
      try {
        window.localStorage.setItem("corescout.theme", next);
      } catch {
        /* a theme that does not persist is survivable */
      }
    },
  ];
}

export default function App() {
  const [screen, setScreen] = useState<ScreenId>("home");
  const [welcoming, setWelcoming] = useState(() => !isOnboarded());
  const [theme, setTheme] = useTheme();
  const status = useCall<Status>("status", {}, 3000);

  if (welcoming) return <Onboarding onDone={() => setWelcoming(false)} />;

  const connected = status.data?.connected ?? [];

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">
          <Mark />
          <span>
            CoreScout
            <br />
            <small>{status.data?.paused ? "paused" : status.data?.autonomy ?? ""}</small>
          </span>
        </div>

        <nav className="nav">
          {SCREENS.map((item) => (
            <button
              key={item.id}
              aria-current={screen === item.id ? "page" : undefined}
              onClick={() => setScreen(item.id)}
            >
              <span className="dot" />
              {item.label}
            </button>
          ))}
        </nav>

        <div className="sidebar-foot">
          <div className="faint" style={{ fontSize: "var(--t-micro)" }}>
            {status.error
              ? "Service not reachable"
              : connected.length > 0
                ? `${connected.join(", ")} connected`
                : "No AI connected"}
          </div>
          <div className="toolbar" style={{ alignSelf: "flex-start" }}>
            {["system", "light", "dark"].map((option) => (
              <button
                key={option}
                aria-pressed={theme === option}
                onClick={() => setTheme(option)}
                title={`${option} theme`}
              >
                {option === "system" ? "Auto" : option === "light" ? "Light" : "Dark"}
              </button>
            ))}
          </div>
        </div>
      </aside>

      <main className="main">
        {screen === "home" ? <Home onGo={(next) => setScreen(next as ScreenId)} /> : null}
        {screen === "ai" ? <AI /> : null}
        {screen === "learned" ? <Learned /> : null}
        {screen === "activity" ? <Activity /> : null}
        {screen === "computer" ? <Computer /> : null}
        {screen === "settings" ? <Settings status={status.data} /> : null}
      </main>
    </div>
  );
}

/**
 * The mark: a filled dot and its reflection, fading.
 *
 * The whole idea in eight lines of SVG, and nothing that looks like a brain.
 */
function Mark() {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
      <circle cx="9" cy="6" r="3.2" fill="var(--accent)" />
      <circle cx="9" cy="13" r="3.2" fill="var(--accent)" opacity="0.28" />
      <line x1="2" y1="9.5" x2="16" y2="9.5" stroke="var(--line-strong)" strokeWidth="1" />
    </svg>
  );
}
