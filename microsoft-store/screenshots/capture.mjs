/**
 * Screenshots of the real application, against a real running service.
 *
 * # Why this is not a mock
 *
 * The page loaded here is the built application, unmodified. The only thing
 * injected is a shim that forwards its one bridge command to the service's
 * loopback API over HTTP, which is exactly what the desktop shell does with
 * the same token. Everything on screen was produced by the real engine from
 * data really fed to it.
 *
 * That matters beyond principle: a Store listing whose screenshots do not
 * match the product is a certification failure and, more to the point, a
 * promise to a buyer that the first launch will break.
 *
 * # Usage
 *
 *   node capture.mjs <endpoint.json> <dist directory> <output directory>
 *
 * `seed.ps1` starts a service, feeds it, and calls this.
 */

import { chromium } from "playwright-core";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { mkdirSync } from "node:fs";
import { extname, join, normalize, resolve, sep } from "node:path";

const [endpointPath, distDir, outDir] = process.argv.slice(2);
if (!endpointPath || !distDir || !outDir) {
  console.error("usage: node capture.mjs <endpoint.json> <dist> <out>");
  process.exit(2);
}

const endpoint = JSON.parse(await readFile(endpointPath, "utf8"));
const root = resolve(distDir);
mkdirSync(outDir, { recursive: true });

const TYPES = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".svg": "image/svg+xml",
  ".png": "image/png",
};

/** Serve the built application. Paths are resolved and confined to `root`. */
const server = createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", "http://localhost");
  const wanted = url.pathname === "/" ? "/index.html" : url.pathname;
  const path = resolve(join(root, normalize(wanted)));
  if (path !== root && !path.startsWith(root + sep)) {
    response.writeHead(403).end();
    return;
  }
  try {
    const body = await readFile(path);
    response.writeHead(200, { "content-type": TYPES[extname(path)] ?? "application/octet-stream" });
    response.end(body);
  } catch {
    response.writeHead(404).end();
  }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const port = server.address().port;

const browser = await chromium.launch();
const page = await browser.newPage({
  viewport: { width: 1366, height: 768 },
  deviceScaleFactor: 2,
});

// The shim. One command, forwarded to the real service exactly as the desktop
// shell forwards it, with the same token.
await page.addInitScript(
  ({ base, token }) => {
    const post = async (method, params) => {
      const response = await fetch(`${base}/call`, {
        method: "POST",
        headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
        body: JSON.stringify({ method, params }),
      });
      const body = await response.json();
      if (!response.ok) throw new Error(body?.error ?? `CoreScout answered ${response.status}`);
      return body;
    };
    const invoke = async (command, args) => {
      if (command === "corescout_call") return post(args.method, args.params ?? {});
      // The shell's own commands. A screenshot must not depend on the state of
      // this machine's registry, so the answer is the one a fresh install
      // gives rather than whatever is true here.
      if (command === "launch_at_login") {
        return {
          enabled: false,
          changeable: true,
          explain:
            "CoreScout only runs while you have it open. It will not learn anything in between.",
        };
      }
      return null;
    };
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: { invoke } });
    Object.defineProperty(window, "__TAURI__", { value: { core: { invoke } } });
    // Past the welcome, and in whichever theme the shot wants.
    window.localStorage.setItem("corescout.onboarded", "yes");
  },
  { base: `http://${endpoint.host}:${endpoint.port}`, token: endpoint.token },
);

const shots = [
  { name: "01-home", nav: "Home", wait: 2600, theme: "light" },
  { name: "02-learned", nav: "Learned", wait: 900, theme: "light" },
  { name: "03-computer", nav: "Computer", wait: 2600, theme: "dark" },
  { name: "04-ai", nav: "AI", wait: 900, theme: "light" },
  { name: "05-activity", nav: "Activity", wait: 900, theme: "dark" },
  { name: "06-plan", nav: "Settings", wait: 900, theme: "light", tab: "Plan" },
  { name: "07-privacy", nav: "Settings", wait: 900, theme: "light", tab: "Privacy" },
  // The prices, which live below the fold on the Plan tab because the record
  // of what this machine worked out comes first and should.
  {
    name: "08-pricing",
    nav: "Settings",
    wait: 900,
    theme: "dark",
    tab: "Plan",
    scrollTo: ".plans",
  },
];

for (const shot of shots) {
  await page.goto(`http://127.0.0.1:${port}/`, { waitUntil: "networkidle" });
  // Set, then reload: the application reads its theme once on mount, so
  // setting it afterwards leaves the toggle showing one thing and the page
  // rendering another.
  await page.evaluate((theme) => {
    window.localStorage.setItem("corescout.theme", theme);
  }, shot.theme);
  await page.reload({ waitUntil: "networkidle" });
  await page.getByRole("button", { name: shot.nav, exact: true }).click();
  if (shot.tab) {
    await page.getByRole("button", { name: shot.tab, exact: true }).click();
  }
  // Clicking a tab can leave the pane scrolled, which crops the heading off
  // the top of the shot.
  await page.evaluate((selector) => {
    window.scrollTo(0, 0);
    for (const element of document.querySelectorAll("*")) element.scrollTop = 0;
    if (selector) {
      document.querySelector(selector)?.scrollIntoView({ block: "center" });
    }
  }, shot.scrollTo ?? null);
  // The mirror animates towards its resting state and the sparkline needs a
  // few samples, so the busier screens are given longer than a paint.
  await page.waitForTimeout(shot.wait);
  await page.screenshot({ path: join(outDir, `${shot.name}.png`) });
  console.log(`  ${shot.name}.png  (${shot.nav}${shot.tab ? " / " + shot.tab : ""}, ${shot.theme})`);
}

await browser.close();
server.close();
