import { chromium } from "playwright-core";
import { existsSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const here = dirname(fileURLToPath(import.meta.url));
function cp(){const r=join(process.env.LOCALAPPDATA,"ms-playwright");
for(const b of readdirSync(r).filter(n=>/^chromium-\d+$/.test(n)).sort((a,b)=>Number(b.split("-")[1])-Number(a.split("-")[1])))
for(const d of ["chrome-win64","chrome-win"]){const e=join(r,b,d,"chrome.exe");if(existsSync(e))return e;}throw new Error("x");}
const ctx = await chromium.launchPersistentContext(join(here,".browser"),{headless:false,executablePath:cp(),viewport:{width:1500,height:1300}});
const page = ctx.pages()[0] ?? await ctx.newPage();
await page.goto("https://partner.microsoft.com/dashboard/products/9PJPNV5VDBV0",{waitUntil:"domcontentloaded"});
await page.waitForTimeout(14000);
// Expand the submission accordion so the per-section states are rendered.
const chev = page.locator('[class*="accordion"] [class*="chevron"], button[aria-expanded="false"]').first();
try { await chev.click({timeout:4000}); await page.waitForTimeout(4000); } catch {}
const t = await page.evaluate(()=>document.body.innerText);
const lines = t.split("\n").map(s=>s.trim()).filter(Boolean);
for (let i=0;i<lines.length;i++) {
  if (/^(Pricing and availability|Properties|Age ratings|Packages|Store listings|Submission options|Certification status|Product submission)/.test(lines[i]))
    console.log(lines[i], "  ->  ", lines[i+1] ?? "");
}
await page.screenshot({path:join(here,"shots","confirm.png"), fullPage:true});
await ctx.close();
