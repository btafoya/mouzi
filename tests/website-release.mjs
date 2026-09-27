import assert from 'node:assert/strict';
import { readFile, readdir, stat } from 'node:fs/promises';
import { resolve, join, extname, relative } from 'node:path';
import { createServer } from 'node:http';
const version = JSON.parse(await readFile('src-tauri/tauri.conf.json', 'utf8')).version;
const root = resolve('website/dist');
async function files(dir) {
  const out = [];
  for (const item of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, item.name);
    out.push(...(item.isDirectory() ? await files(path) : [path]));
  }
  return out;
}
const all = await files(root), html = all.filter(f => f.endsWith('.html'));
async function locate(path) {
  const full = resolve(root, `.${decodeURIComponent(path)}`);
  if (!full.startsWith(root)) return null;
  for (const candidate of [full, `${full}.html`, join(full, 'index.html')]) {
    if (await stat(candidate).then(s => s.isFile(), () => false)) return candidate;
  }
  return null;
}
let checked = 0;
for (const file of html) {
  const content = await readFile(file, 'utf8');
  for (const match of content.matchAll(/(?:href|src)="([^"\s]+)"/g)) {
    const link = match[1];
    if (!link.startsWith('/') || link.startsWith('//')) continue;
    const url = new URL(link, 'https://mouzi.cc');
    assert.ok(await locate(url.pathname), `${relative(root, file)} has a broken link: ${link}`);
    checked++;
  }
}
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.json': 'application/json', '.svg': 'image/svg+xml' };
const server = createServer(async (req, res) => {
  const path = await locate(new URL(req.url, 'http://localhost').pathname);
  if (!path) { res.writeHead(404).end(); return; }
  res.setHeader('Content-Type', mime[extname(path)] || 'application/octet-stream');
  res.end(await readFile(path));
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const { chromium } = await import(process.env.MOUZI_PLAYWRIGHT_MODULE || 'playwright');
let browser;
try {
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}/docs`);
  const results = await page.evaluate(async () => {
    const pagefind = await import('/pagefind/pagefind.js');
    const found = await pagefind.search('Suggest');
    return Promise.all(found.results.map(r => r.data()));
  });
  assert.ok(results.some(r => r.url.includes('organization-preview')), 'New preview documentation missing from search');
  await page.goto(`http://127.0.0.1:${server.address().port}/download/windows`);
  assert.ok((await page.content()).includes(`Mouzi_${version}_x64-setup.exe`));
  for (const locale of ['', '/pl', '/es', '/de', '/fr', '/it']) {
    await page.goto(`http://127.0.0.1:${server.address().port}${locale}/changelog`);
    assert.ok((await page.content()).includes(version), `Missing release in ${locale} changelog`);
    assert.equal((await page.locator('article h2').first().textContent()).trim(), `v${version}`, `Latest release must appear first in ${locale} changelog`);
  }
  console.log(`PASS ${html.length} pages, ${checked} internal references, Pagefind results and all localized changelogs`);
} finally {
  await browser?.close();
  server.close();
}
