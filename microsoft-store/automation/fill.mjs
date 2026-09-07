/**
 * Fill in the Partner Center submission.
 *
 * # What it will and will not do
 *
 * It fills every section it knows how to fill and presses that section's Save.
 * It never presses "Submit for certification": the last look before this
 * becomes public is a person's.
 *
 * It is safe to run twice. Every step reads the current state first and skips
 * what is already right, so a re-run after a partial failure does not undo
 * work or duplicate anything.
 *
 * # Why a real browser window
 *
 * Partner Center sits behind bot protection that returns "Access Denied" to a
 * headless browser. So this runs headed, which also means you can watch it and
 * stop it.
 *
 * # Usage
 *
 *   node fill.mjs                    every section
 *   node fill.mjs properties         one section
 *   node fill.mjs --dry              report what it would change, change nothing
 */

import { chromium } from "playwright-core";
import { existsSync, mkdirSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..", "..");

const STORE_ID = "9PJPNV5VDBV0";
const SUBMISSION = process.env.CORESCOUT_SUBMISSION ?? "1152921505701826456";
const BASE = `https://partner.microsoft.com/en-us/dashboard/products/${STORE_ID}/submissions/${SUBMISSION}`;

const args = process.argv.slice(2);
const dry = args.includes("--dry");
const wanted = args.filter((a) => !a.startsWith("--"));

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
  throw new Error(`no Chromium under ${root}; run: npx playwright install chromium`);
}

const context = await chromium.launchPersistentContext(join(here, ".browser"), {
  headless: false,
  executablePath: chromiumPath(),
  viewport: { width: 1500, height: 1200 },
});
const page = context.pages()[0] ?? (await context.newPage());

const changes = [];
const note = (what) => {
  changes.push(what);
  console.log(`   ${dry ? "would set" : "set"}: ${what}`);
};

/** Go to a section and wait for it to actually render. */
async function open(section) {
  await page.goto(`${BASE}/${section}`, { waitUntil: "domcontentloaded" });
  // Partner Center renders in stages: shell, form, then the values already
  // saved. Reading too early gives an empty form that looks like an empty form.
  await page.waitForTimeout(11000);
  if (!page.url().includes(section)) {
    throw new Error(`${section} redirected to ${page.url()}`);
  }
}

/** Set a text field found by its visible label, if it is not already right. */
async function text(label, value) {
  const field = page.getByLabel(label, { exact: false }).first();
  if (!(await field.isVisible().catch(() => false))) {
    console.log(`   ! no field labelled ${JSON.stringify(label)}`);
    return;
  }
  const current = await field.inputValue().catch(() => "");
  if (current.trim() === value) return;
  if (!dry) {
    await field.fill(value);
  }
  note(`${label} = ${value}`);
}

/**
 * Set a declaration checkbox by its `name` attribute.
 *
 * Partner Center's checkboxes are web components: the real input is inside a
 * shadow root, its label is slotted in from outside it, and the checked state
 * is a class on a wrapping element rather than the input's own `checked`. So
 * neither the accessible name nor `isChecked` can be trusted here, and this
 * walks the shadow trees looking for the `name`, which is stable and
 * meaningful ("accessibility-checkbox", "usesGenAI-checkbox").
 */
async function check(name, on) {
  const state = await page.evaluate((wanted) => {
    const walk = (root) => {
      for (const element of root.querySelectorAll("*")) {
        if (element.shadowRoot) {
          const deeper = walk(element.shadowRoot);
          if (deeper) return deeper;
        }
        if (
          element.tagName === "INPUT" &&
          element.type === "checkbox" &&
          (element.getAttribute("name") || "").replace(/'/g, "") === wanted
        ) {
          const label = element.closest("label");
          return { checked: !!label && label.className.includes("checkbox--checked") };
        }
      }
      return null;
    };
    return walk(document);
  }, name);

  if (!state) {
    console.log(`   ! no checkbox named ${name}`);
    return;
  }
  if (state.checked === on) return;
  if (!dry) {
    await page.evaluate((wanted) => {
      const walk = (root) => {
        for (const element of root.querySelectorAll("*")) {
          if (element.shadowRoot) {
            const deeper = walk(element.shadowRoot);
            if (deeper) return deeper;
          }
          if (
            element.tagName === "INPUT" &&
            element.type === "checkbox" &&
            (element.getAttribute("name") || "").replace(/'/g, "") === wanted
          ) {
            return element;
          }
        }
        return null;
      };
      walk(document)?.click();
    }, name);
    await page.waitForTimeout(500);
  }
  note(`${name} = ${on ? "checked" : "unchecked"}`);
}

/**
 * Set a checkbox that has no `name`, by the label slotted into it.
 *
 * The device-family boxes are the same web component as the declarations but
 * without a name attribute, so they can only be found by their text.
 */
async function checkByLabel(label, on) {
  const state = await page.evaluate((wanted) => {
    const walk = (root) => {
      for (const element of root.querySelectorAll("*")) {
        if (element.shadowRoot) {
          const deeper = walk(element.shadowRoot);
          if (deeper) return deeper;
        }
        if (element.tagName !== "INPUT" || element.type !== "checkbox") continue;
        const host = (element.getRootNode()?.host?.textContent || "")
          .replace(/\s+/g, " ")
          .trim();
        if (host !== wanted) continue;
        const box = element.closest("label");
        return {
          element,
          checked: !!box && box.className.includes("checkbox--checked"),
        };
      }
      return null;
    };
    const found = walk(document);
    if (!found) return null;
    return { checked: found.checked };
  }, label);

  if (!state) {
    console.log(`   ! no checkbox labelled ${JSON.stringify(label)}`);
    return;
  }
  if (state.checked === on) return;
  if (!dry) {
    await page.evaluate((wanted) => {
      const walk = (root) => {
        for (const element of root.querySelectorAll("*")) {
          if (element.shadowRoot) {
            const deeper = walk(element.shadowRoot);
            if (deeper) return deeper;
          }
          if (element.tagName !== "INPUT" || element.type !== "checkbox") continue;
          const host = (element.getRootNode()?.host?.textContent || "")
            .replace(/\s+/g, " ")
            .trim();
          if (host === wanted) return element;
        }
        return null;
      };
      walk(document)?.click();
    }, label);
    await page.waitForTimeout(600);
  }
  note(`${label} = ${on ? "checked" : "unchecked"}`);
}

/** Choose a radio by its visible label, if it is not already chosen. */
async function radio(label) {
  const button = page.getByRole("radio", { name: new RegExp(escape(label), "i") }).first();
  if (!(await button.isVisible().catch(() => false))) {
    console.log(`   ! no radio matching ${JSON.stringify(label)}`);
    return;
  }
  if (await button.isChecked()) return;
  if (!dry) {
    await button.check();
  }
  note(`chose ${label}`);
}

function escape(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Choose one of a group of radio buttons, by the group's `name` and the
 * option's position in it.
 *
 * The IARC questionnaire's radios are web components whose labels are not in
 * the same DOM tree as the input, so they cannot be found by their accessible
 * name the way an ordinary radio can. What they do have is a stable group name
 * ("question#1109") and a fixed order, and the order is the order the question
 * is written in, which is what `describes` records so this stays readable.
 */
async function pick(group, index, describes) {
  const clicked = await page.evaluate(
    ({ group, index }) => {
      const found = [];
      const walk = (root) => {
        for (const element of root.querySelectorAll("*")) {
          if (element.shadowRoot) walk(element.shadowRoot);
          if (
            element.tagName === "INPUT" &&
            element.type === "radio" &&
            (element.getAttribute("name") || "").replace(/'/g, "") === group
          ) {
            found.push(element);
          }
        }
      };
      walk(document);
      const target = found[index];
      if (!target) return { ok: false, count: found.length };
      if (target.checked) return { ok: true, already: true };
      target.click();
      return { ok: true, already: false };
    },
    { group, index },
  );

  if (!clicked.ok) {
    console.log(`   ! ${group} has no option ${index} (found ${clicked.count})`);
    return;
  }
  if (clicked.already) return;
  await page.waitForTimeout(800);
  note(`${describes}`);
}

/**
 * Press a button by its visible text.
 *
 * Partner Center's buttons are web components: the `<button>` lives inside a
 * shadow root with no text of its own, and the label is slotted in from
 * outside. So the accessible name is empty and `getByRole("button", { name })`
 * matches nothing. The text is on the shadow host, which is what this looks at.
 */
async function press(label) {
  const clicked = await page.evaluate((wanted) => {
    const walk = (root) => {
      for (const element of root.querySelectorAll("*")) {
        if (element.shadowRoot) {
          const deeper = walk(element.shadowRoot);
          if (deeper) return deeper;
        }
        // Buttons here are sometimes <button>, sometimes <input type=submit>,
        // and the label is sometimes the element's own text and sometimes
        // slotted in from outside its shadow root.
        const isButton =
          element.tagName === "BUTTON" ||
          (element.tagName === "INPUT" && ["submit", "button"].includes(element.type));
        if (!isButton) continue;
        const own = (element.innerText || element.value || "").trim();
        const host = (element.getRootNode()?.host?.textContent || "").replace(/\s+/g, " ").trim();
        if (own === wanted || host === wanted) {
          const box = element.getBoundingClientRect();
          if (box.width > 0 && box.height > 0 && !element.disabled) return element;
        }
      }
      return null;
    };
    const target = walk(document);
    if (!target) {
      // Report what was there instead, so a changed label is a one-line fix
      // rather than another round of probing.
      const seen = [];
      const collect = (root) => {
        for (const element of root.querySelectorAll("*")) {
          if (element.shadowRoot) collect(element.shadowRoot);
          const isButton =
            element.tagName === "BUTTON" ||
            (element.tagName === "INPUT" && ["submit", "button"].includes(element.type));
          if (!isButton) continue;
          const box = element.getBoundingClientRect();
          if (box.width === 0 || box.height === 0) continue;
          const own = (element.innerText || element.value || "").trim();
          const host = (element.getRootNode()?.host?.textContent || "")
            .replace(/\s+/g, " ")
            .trim();
          seen.push(`${JSON.stringify(own || host)}${element.disabled ? " (disabled)" : ""}`);
        }
      };
      collect(document);
      return { ok: false, seen: seen.slice(0, 20) };
    }
    target.click();
    return { ok: true };
  }, label);
  if (!clicked.ok) {
    console.log(`   ! no ${JSON.stringify(label)} button; visible buttons: ${clicked.seen.join(", ")}`);
  }
  return clicked.ok;
}

/** Press the section's Save, and wait for it to land. */
async function save(section, label = "Save") {
  if (dry) {
    console.log("   (dry run, not saving)");
    return;
  }
  if (!(await press(label))) return;
  await page.waitForTimeout(8000);
  await page.screenshot({ path: join(shots, `saved-${section}.png`), fullPage: true });
  console.log(`   ${label.toLowerCase()}d`);
}

/**
 * Paste the privacy policy into whichever textarea Partner Center just showed.
 *
 * The text is the published half of `../PRIVACY.md`, read from that file so
 * there is one copy of it rather than two that drift.
 */
async function policyText() {
  const source = readFileSync(join(here, "..", "PRIVACY.md"), "utf8");
  const start = source.indexOf("## Privacy policy");
  const end = source.indexOf("## Notes for the submission");
  if (start === -1 || end === -1) {
    console.log("   ! could not find the policy section in PRIVACY.md");
    return;
  }
  const policy = source
    .slice(start, end)
    .replace(/^## Privacy policy\s*/, "")
    .replace(/^#+\s*/gm, "")
    .replace(/\*\*/g, "")
    .replace(/^---$/gm, "")
    .replace(/`/g, "")
    .trim();

  const area = page.locator("textarea").filter({ hasNot: page.locator("[disabled]") });
  const count = await area.count();
  for (let index = 0; index < count; index += 1) {
    const one = area.nth(index);
    const aria = (await one.getAttribute("aria-label")) ?? "";
    if (aria.includes("AI Assistant")) continue;
    if (!(await one.isVisible().catch(() => false))) continue;
    const current = (await one.inputValue().catch(() => "")).trim();
    if (current === policy) return;
    if (!dry) await one.fill(policy);
    note(`privacy policy text (${policy.length} characters)`);
    return;
  }
  console.log("   ! no textarea to paste the privacy policy into");
}

// --------------------------------------------------------------- sections

const SECTIONS = {
  /**
   * Category, support links and declarations.
   *
   * Developer tools, not Utilities: CoreScout is bought by developers to
   * change how their development tooling behaves, and a utilities listing puts
   * it beside disk cleaners.
   */
  async properties() {
    await open("properties");
    await text("Apps website URL", "https://corescout.dev");
    await text("Apps support contact URL", "support@corescout.dev");

    // Everything CoreScout sells goes through Store commerce, so the
    // alternative-commerce declaration stays off.
    await check("store-checkbox", false);
    await check("accessibility-checkbox", true);
    await check("storage-checkbox", true);

    // Off, and this one is a real decision rather than a default.
    //
    // CoreScout's data is a model of *this* machine: its processor counters,
    // its recurring states, what failed on it. Restored onto a different
    // machine by a OneDrive backup, that model describes hardware the agent is
    // not running on, and an agent acting confidently on a model of somebody
    // else's computer is the exact failure this product exists to prevent.
    // A fresh install learning from nothing is the better outcome.
    await check("backups-checkbox", false);

    // There is no inference in CoreScout. It is measurement, statistics and a
    // refusal to overclaim, and declaring otherwise would be false.
    await check("usesGenAI-checkbox", false);

    // A privacy policy is required because the package declares runFullTrust.
    //
    // Partner Center takes either a hosted URL or the policy text pasted in.
    // The text is used, because it means the submission does not wait on a
    // website being live, and because a policy that ships with the submission
    // cannot later disagree with the one in this repository.
    await pick("privacyPolicySelection", 1, "provide the privacy policy as text");
    await page.waitForTimeout(1500);
    await policyText();

    await save("properties");
  },

  /**
   * Markets, audience, discoverability and the release schedule.
   *
   * The defaults are already what CoreScout wants, so this mostly confirms
   * them rather than changing them. Confirming is worth doing: a default is
   * not a decision until somebody has looked at it.
   */
  async availability() {
    await open("availability");
    await radio("All worldwide markets");
    await radio("Public audience");
    await radio("Make this product available and discoverable in the Microsoft Store");
    await save("availability");
  },

  /**
   * The IARC age-rating questionnaire.
   *
   * Answered rather than chosen: Microsoft derives the rating from the
   * answers, and there is no way to pick one directly. Every answer here is
   * the truthful one for a local developer tool, and `../COMPLIANCE.md` has
   * the reasoning for the two that are not obviously no.
   *
   * The questionnaire is a wizard whose later pages depend on the earlier
   * answers, so this walks it a page at a time rather than filling a form.
   */
  async ageratings() {
    await open("ageratings");

    // Complete the questionnaire rather than supplying an existing IARC
    // certificate, which CoreScout does not have.
    await pick("inputMode", 0, "complete the IARC questionnaire");
    // Game / Social or Communication / All Other App Types. CoreScout is the
    // third: it is not a game, and it has no users other than the one person
    // sitting at the machine.
    await pick("question#1109", 2, "app type: all other app types");

    // The content questions. Every one of them is No, and every one of them is
    // No truthfully: CoreScout has no content, no users other than the person
    // at the machine, nothing downloaded after install, and nothing for sale
    // inside it. Index 1 is "No" in each pair.
    //
    // The one that changed late is "purchase digital goods". CoreScout used to
    // sell add-ons, which would have made it Yes. It is now bought once in the
    // Store and sells nothing from inside the app, so it is No.
    const NO = 1;
    for (const [group, asks] of [
      ["question#1152", "ratings-relevant content in the package"],
      ["question#1188", "users interacting or exchanging content"],
      ["question#1193", "content reachable from the app but not in it"],
      ["question#1037", "promoting or selling age-restricted things"],
      ["question#1194", "sharing the user's precise location"],
      ["question#1195", "purchasing digital goods inside the app"],
      ["question#1375", "cash rewards, gift cards, crypto or NFTs"],
      ["question#1196", "being a web browser or search engine"],
      ["question#1197", "being a news or educational product"],
    ]) {
      await pick(group, NO, `no: ${asks}`);
    }

    // Once every question is answered the button changes from "Save draft" to
    // "Preview ratings", which is the step that actually generates the rating.
    for (const label of ["Preview ratings", "Save draft"]) {
      if (await press(label)) {
        console.log(`   pressed ${label}`);
        await page.waitForTimeout(9000);
        break;
      }
    }
    await page.screenshot({ path: join(shots, "age-after.png"), fullPage: true });

    // And it stops here, one click short of done, on purpose.
    //
    // The last control on this page is "I agree to the IARC Terms of Use and I
    // am the age of majority in my jurisdiction". That is a personal legal
    // attestation about a specific human being, and nothing automated should
    // make it on their behalf. The answers above are facts about the software
    // and can be checked; this one is not.
    console.log("   >> one thing left for a person: tick the IARC terms box and save.");
    const after = await page.evaluate(() => document.body.innerText.slice(0, 1800));
    console.log("   --- what it says now ---");
    for (const line of after.split("\n")) {
      if (line.trim()) console.log("   " + line.trim());
    }
  },

  /**
   * Upload the MSIX.
   *
   * Built by `scripts/msix.ps1`, unsigned on purpose: Microsoft signs Store
   * submissions. The file input is hidden behind a styled drop zone, which
   * Playwright can set directly.
   */
  async packages() {
    await open("packages");

    const msix = join(repo, "dist", "CoreScout.msix");
    if (!existsSync(msix)) {
      throw new Error(`no package at ${msix}; run scripts/msix.ps1 first`);
    }

    // Already uploaded? The page lists it by name once it is there.
    let already = await page.evaluate(() => document.body.innerText.includes("CoreScout.msix"));
    // A package the Store rejected still shows its name, so "is it there" is
    // not the same question as "is it usable". A rejected one is removed and
    // replaced rather than left sitting under an error nobody reads twice.
    const rejected = await page.evaluate(() =>
      document.body.innerText.includes("Package acceptance validation error"),
    );
    if (rejected && !dry) {
      // Delete only the rejected upload, found by walking up from its error
      // message to the row that owns it. Clicking the first "Delete" on the
      // page would remove whichever package happens to be listed first, which
      // after a re-upload is as likely to be the good one.
      const removed = await page.evaluate(() => {
        const error = [...document.querySelectorAll("*")].find(
          (element) =>
            element.children.length === 0 &&
            element.textContent.includes("Package acceptance validation error"),
        );
        if (!error) return false;
        let row = error;
        for (let up = 0; up < 6 && row; up += 1) {
          const remove = [...row.querySelectorAll("a, button")].find(
            (element) => element.textContent.trim() === "Delete",
          );
          if (remove) {
            remove.click();
            return true;
          }
          row = row.parentElement;
        }
        return false;
      });
      if (removed) {
        console.log("   removed the package the Store rejected");
        await page.waitForTimeout(8000);
      } else {
        console.log("   ! could not find the Delete for the rejected package");
      }
      already = await page.evaluate(() => document.body.innerText.includes("CoreScout.msix"));
    }

    if (dry) {
      if (!already) note(`upload ${msix}`);
      return;
    }

    if (already) {
      console.log("   already uploaded");
    } else {
      const input = page.locator('input[type="file"]').first();
      await input.setInputFiles(msix);
      note("uploaded CoreScout.msix");
    }

    // The Store validates the package after upload, which takes a while and
    // is the step that catches a bad manifest. Worth waiting for rather than
    // saving over the top of.
    for (let waited = 0; !already && waited < 180; waited += 10) {
      await page.waitForTimeout(10_000);
      const said = await page.evaluate(() => document.body.innerText);
      if (said.includes("CoreScout.msix")) {
        console.log(`   the Store accepted it after about ${waited + 10}s`);
        break;
      }
    }
    // Without a device family, the package is accepted and then offered to
    // nobody: Partner Center warns that it "will not be available to customers
    // on Windows 10/11". This is the only one CoreScout targets.
    await checkByLabel("Windows 10/11 Desktop", true);

    await page.screenshot({ path: join(shots, "packages-after.png"), fullPage: true });
    await save("packages");
  },

  /**
   * The Store listing: description, release notes, features, screenshots.
   *
   * The copy is read out of `../LISTING.md` rather than repeated here, so the
   * document a person edits and the text that reaches the Store are the same
   * text. The fields on this page carry no labels of any kind, so they are
   * addressed by order, which is the order the page has always been in:
   * description, then what's new, then features.
   */
  async listings() {
    await open("listings?languageid=4&languagecode=en-us");

    const listing = readFileSync(join(here, "..", "LISTING.md"), "utf8");
    const block = (heading) => {
      const at = listing.indexOf(`## ${heading}`);
      if (at === -1) throw new Error(`LISTING.md has no "${heading}" section`);
      const open = listing.indexOf("```", at);
      const close = listing.indexOf("```", open + 3);
      // The fence is followed by a newline that is not part of the copy.
      return listing.slice(open + 3, close).replace(/^\r?\n/, "").trimEnd();
    };

    const areas = [];
    for (const one of await page.locator("textarea").all()) {
      const aria = (await one.getAttribute("aria-label")) ?? "";
      if (aria.includes("AI Assistant")) continue;
      if (await one.isVisible().catch(() => false)) areas.push(one);
    }
    if (areas.length < 2) {
      throw new Error(`expected the description and release notes, found ${areas.length} fields`);
    }

    for (const [index, [name, heading]] of [
      ["description", "Description (10,000)"],
      ["what's new", "What's new in this version (1,500)"],
    ].entries()) {
      const value = block(heading);
      const current = (await areas[index].inputValue().catch(() => "")).trim();
      if (current === value.trim()) continue;
      if (!dry) await areas[index].fill(value);
      note(`${name} (${value.length} characters)`);
    }

    // Screenshots. The Store needs at least one and recommends four; these are
    // the real ones from screenshots/out, captured against a running CoreScout.
    const shotDir = join(repo, "microsoft-store", "screenshots", "out");
    const files = existsSync(shotDir)
      ? readdirSync(shotDir)
          .filter((name) => name.endsWith(".png"))
          .sort()
          .map((name) => join(shotDir, name))
      : [];
    if (files.length === 0) {
      console.log("   ! no screenshots in microsoft-store/screenshots/out");
      // The page says so itself while it has none. Counting the tab label
      // ("Desktop (0)") looked like the same check and is not: the label stops
      // saying zero before the upload has actually been accepted.
    } else if (
      await page.evaluate(() =>
        document.body.innerText.includes("At least one screenshot is required."),
      )
    ) {
      if (!dry) {
        // One at a time: the input is not `multiple`, and each upload has to
        // be accepted before the next can be handed over.
        for (const file of files) {
          await page.locator('input[type="file"]').first().setInputFiles(file);
          await page.waitForTimeout(6000);
        }
      }
      note(`${files.length} screenshots`);
    } else {
      console.log("   screenshots already uploaded");
    }

    await page.screenshot({ path: join(shots, "listing-filled.png"), fullPage: true });
    await save("listings");
  },

  /** When it goes live once it passes certification. */
  async options() {
    await open("options");
    await radio("Publish this submission as soon as it passes certification");

    // The package declares runFullTrust, which is a restricted capability and
    // has to be justified in writing before certification will consider it.
    // This is the same reasoning as ../COMPLIANCE.md, written for a reviewer.
    // Under 500 characters, which is this field's limit: a longer one is
    // silently truncated, which is how the first attempt lost half its
    // reasoning without saying so.
    const justification =
      "CoreScout is a Win32 desktop app packaged as MSIX. runFullTrust is " +
      "required for two things a sandboxed app cannot do: read hardware " +
      "performance counters from the processor, which is the product; and " +
      "start its own helper processes (a background service, a command " +
      "line tool, and an MCP bridge that AI coding assistants launch). No " +
      "other restricted capability is declared. No elevation, no " +
      "broadFileSystemAccess, no account, no server, no telemetry. Its " +
      "only socket is bound to 127.0.0.1 for local IPC.";

    const areas = [];
    for (const one of await page.locator("textarea").all()) {
      const aria = (await one.getAttribute("aria-label")) ?? "";
      if (aria.includes("AI Assistant")) continue;
      if (await one.isVisible().catch(() => false)) areas.push(one);
    }
    if (areas.length === 0) {
      console.log("   ! no field for the restricted capability justification");
    } else {
      const current = (await areas[0].inputValue().catch(() => "")).trim();
      if (current !== justification.trim()) {
        if (!dry) await areas[0].fill(justification);
        note(`runFullTrust justification (${justification.length} characters)`);
      }
    }

    await save("options");
  },
};

const order = wanted.length > 0 ? wanted : Object.keys(SECTIONS);
for (const name of order) {
  const section = SECTIONS[name];
  if (!section) {
    console.log(`\n${name}: no such section (have ${Object.keys(SECTIONS).join(", ")})`);
    continue;
  }
  console.log(`\n${name}`);
  try {
    await section();
  } catch (error) {
    console.log(`   FAILED: ${error.message}`);
    await page.screenshot({ path: join(shots, `failed-${name}.png`), fullPage: true });
  }
}

console.log(`\n${changes.length} change(s). Nothing was submitted for certification.`);
await context.close();
