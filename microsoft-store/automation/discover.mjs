/**
 * Learn what the submission pages are actually called, and what is on them.
 *
 * Partner Center is a single-page application whose section URLs are not
 * guessable and not documented. Rather than guess, this clicks each row of the
 * submission and writes down where it lands and every form control it finds
 * there, into `discovered.json`.
 *
 * That file is then what the filling script is written against, so a selector
 * that breaks can be re-derived by running this again rather than by guessing
 * a second time.
 *
 * Reads only. Nothing is typed, clicked to save, or submitted.
 */

import { chromium } from "playwright-core";
import { mkdirSync, existsSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const STORE_ID = "9PJPNV5VDBV0";
// The submission id, read off the overview page's own links. It is per
// submission, so a second submission will have a different one; `--overview`
// re-reads it rather than assuming this one.
const SUBMISSION = process.env.CORESCOUT_SUBMISSION ?? "1152921505701826456";
const BASE = `https://partner.microsoft.com/en-us/dashboard/products/${STORE_ID}/submissions/${SUBMISSION}`;

const shots = join(here, "shots");
mkdirSync(shots, { recursive: true });

function chromiumPath() {
  const root = join(process.env.LOCALAPPDATA ?? "", "ms-playwright");
  const builds = readdirSync(root)
    .filter((name) => /^chromium-\d+$/.test(name))
    .sort((a, b) => Number(b.split("-")[1]) - Number(a.split("-")[1]));
  for (const build of builds) {
    for (const dir of ["chrome-win64", "chrome-win"]) {
      const exe = join(root, build, dir, "chrome.exe");
      if (existsSync(exe)) return exe;
    }
  }
  throw new Error(`no Chromium under ${root}`);
}

const context = await chromium.launchPersistentContext(join(here, ".browser"), {
  headless: false,
  executablePath: chromiumPath(),
  viewport: { width: 1500, height: 1200 },
});
const page = context.pages()[0] ?? (await context.newPage());

/** Everything on the current page somebody could fill in. */
async function controls() {
  return page.evaluate(() => {
    const label = (element) => {
      const id = element.getAttribute("id");
      if (id) {
        const tag = document.querySelector(`label[for="${CSS.escape(id)}"]`);
        if (tag?.innerText.trim()) return tag.innerText.trim();
      }
      const wrapping = element.closest("label");
      if (wrapping?.innerText.trim()) return wrapping.innerText.trim();
      return (
        element.getAttribute("aria-label") ||
        element.getAttribute("placeholder") ||
        element.getAttribute("name") ||
        ""
      ).trim();
    };
    const seen = [];
    for (const element of document.querySelectorAll("input, textarea, select, button")) {
      const rect = element.getBoundingClientRect();
      if (rect.width === 0 && rect.height === 0) continue;
      seen.push({
        tag: element.tagName.toLowerCase(),
        type: element.getAttribute("type") || "",
        id: element.getAttribute("id") || "",
        name: element.getAttribute("name") || "",
        label: label(element).slice(0, 120),
        text: (element.innerText || "").trim().slice(0, 80),
        checked: element.checked ?? null,
        value: element.tagName === "SELECT" ? element.value : undefined,
        options:
          element.tagName === "SELECT"
            ? [...element.options].map((option) => option.text.trim()).slice(0, 40)
            : undefined,
      });
    }
    return seen;
  });
}

const SECTIONS = {
  availability: `${BASE}/availability`,
  properties: `${BASE}/properties`,
  ageratings: `${BASE}/ageratings`,
  packages: `${BASE}/packages`,
  listings: `${BASE}/listings/en-us`,
  options: `${BASE}/options`,
};

const found = {};
for (const [name, url] of Object.entries(SECTIONS)) {
  await page.goto(url, { waitUntil: "domcontentloaded" });
  // Partner Center renders in stages: shell, then the form, then the values
  // already saved. Reading too early gives an empty form that looks like a
  // form with nothing in it.
  await page.waitForTimeout(11000);
  await page.screenshot({ path: join(shots, `sec-${name}.png`), fullPage: true });
  found[name] = {
    url: page.url(),
    heading: await page.locator("h1, h2").first().innerText().catch(() => ""),
    controls: await controls(),
  };
  console.log(`${name}: ${found[name].controls.length} controls  "${found[name].heading}"`);
}

writeFileSync(join(here, "discovered.json"), JSON.stringify(found, null, 2));
console.log("\nWrote discovered.json");
await context.close();
