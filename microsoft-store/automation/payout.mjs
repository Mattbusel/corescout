import { chromium } from "playwright-core";
import { existsSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const here = dirname(fileURLToPath(import.meta.url));
function cp(){const r=join(process.env.LOCALAPPDATA,"ms-playwright");
for(const b of readdirSync(r).filter(n=>/^chromium-\d+$/.test(n)).sort((a,b)=>Number(b.split("-")[1])-Number(a.split("-")[1])))
for(const d of ["chrome-win64","chrome-win"]){const e=join(r,b,d,"chrome.exe");if(existsSync(e))return e;}throw new Error("x");}
const ctx = await chromium.launchPersistentContext(join(here,".browser"),{headless:false,executablePath:cp(),viewport:{width:1500,height:1100}});
const page = ctx.pages()[0] ?? await ctx.newPage();
// Follow the banner rather than guessing a URL.
await page.goto("https://partner.microsoft.com/dashboard/products/9PJPNV5VDBV0",{waitUntil:"domcontentloaded"});
await page.waitForTimeout(12000);
const link = page.getByText("tax and payout", {exact:false}).first();
if (await link.isVisible().catch(()=>false)) {
  const a = await link.evaluate(e => { const l = e.closest("a") || e.querySelector("a"); return l ? l.href : null; });
  console.log("banner link:", a);
}
await page.goto("https://partner.microsoft.com/dashboard/account/v3/myaccess", { waitUntil: "domcontentloaded" });
await page.waitForTimeout(11000);
const links = await page.evaluate(() =>
  [...document.querySelectorAll("a[href]")]
    .map((a) => ({ t: a.innerText.replace(/\s+/g, " ").trim().slice(0, 46), h: a.getAttribute("href") }))
    .filter((x) => x.h && x.h.includes("/dashboard/")),
);
const seen = new Set();
for (const l of links) {
  if (seen.has(l.h)) continue;
  seen.add(l.h);
  console.log(`${JSON.stringify(l.t).padEnd(50)} ${l.h}`);
}
await ctx.close();
