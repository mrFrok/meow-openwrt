// meow-openwrt — open a dashboard (zashboard / metacubexd, served by the core's
// external-ui) in headless Firefox, walk its pages, print console errors and
// failed or >=400 requests, save screenshots. Run once per core to compare.
// Needs: npm i puppeteer-core; firefox.
// usage: node dashcheck.mjs <zash|mcd> <controller-port> <secret> <outdir>
import puppeteer from 'puppeteer-core';
import fs from 'node:fs';

const [kind, port, secret, out] = process.argv.slice(2);
fs.mkdirSync(out, { recursive: true });
const base = `http://127.0.0.1:${port}/ui/`;
const q = `hostname=127.0.0.1&port=${port}&secret=${secret}`;
const pages = kind === 'zash'
  ? ['#/setup?' + q, '#/proxies', '#/connections', '#/rules', '#/logs', '#/overview', '#/settings']
  : ['#/setup?' + q, '#/proxies', '#/rules', '#/conns', '#/logs', '#/overview', '#/config'];

const browser = await puppeteer.launch({
  browser: 'firefox', executablePath: '/usr/bin/firefox', headless: true,
});
const page = await browser.newPage();
await page.setViewport({ width: 1280, height: 900 });
const log = [];
const note = (k, m) => { const l = `[${k}] ${m}`.slice(0, 400); if (!log.includes(l)) log.push(l); };
page.on('console', (m) => { if (m.type() === 'error' || m.type() === 'warn') note('console.' + m.type(), m.text()); });
page.on('pageerror', (e) => note('pageerror', e.message));
page.on('requestfailed', (r) => note('requestfailed', `${r.method()} ${r.url()} ${r.failure()?.errorText ?? ''}`));
page.on('response', (r) => { if (r.status() >= 400) note('http' + r.status(), `${r.request().method()} ${r.url()}`); });

for (const [i, p] of pages.entries()) {
  note('nav', p);
  try {
    await page.goto(base + p, { waitUntil: 'load', timeout: 20000 });
  } catch (e) { note('goto', e.message); }
  await new Promise((r) => setTimeout(r, i === 0 ? 4000 : 2500));
  await page.screenshot({ path: `${out}/${i}-${p.replace(/[^a-z]/gi, '').slice(0, 20) || 'root'}.png` });
}
fs.writeFileSync(`${out}/log.txt`, log.join('\n') + '\n');
console.log(log.join('\n'));
await browser.close();
