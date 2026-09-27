import assert from 'node:assert/strict';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
const metadata = JSON.parse(await readFile('artifacts/0.2.0/artifact-metadata.json', 'utf8'));
const pages = await Promise.all(['/download/windows', '/download/linux', '/changelog', '/pl/changelog', '/docs/organization-preview', '/'].map(async path => {
  const response = await fetch(`https://mouzi.cc${path}`, { headers: { 'Cache-Control': 'no-cache' } });
  assert.equal(response.status, 200, path);
  const html = await response.text();
  assert.ok(html.includes('0.2.0'), `${path} missing version`);
  return { path, html };
}));
const downloadHtml = pages.filter(p => p.path.startsWith('/download/')).map(p => p.html).join('\n');
const results = [];
for (const artifact of metadata) {
  assert.ok(downloadHtml.includes(artifact.url), `Website missing URL: ${artifact.file}`);
  assert.ok(downloadHtml.includes(artifact.sha256), `Website missing checksum: ${artifact.file}`);
  const response = await fetch(artifact.url);
  assert.equal(response.status, 200, artifact.file);
  const hash = createHash('sha256');
  let size = 0;
  for await (const chunk of response.body) { hash.update(chunk); size += chunk.length; }
  const digest = hash.digest('hex');
  assert.equal(size, artifact.bytes, artifact.file);
  assert.equal(digest, artifact.sha256, artifact.file);
  results.push({ file: artifact.file, size, sha256: digest, status: 'PASS' });
  console.log(`PASS published artifact ${artifact.file}`);
}
const redirect = await fetch('https://www.mouzi.cc/docs/organization-preview?release=0.2.0', { redirect: 'manual' });
assert.ok([301, 308].includes(redirect.status), 'www redirect must be permanent');
assert.equal(redirect.headers.get('location'), 'https://mouzi.cc/docs/organization-preview?release=0.2.0');
const manifestResponse = await fetch('https://github.com/hsr88/mouzi/releases/latest/download/latest.json');
assert.equal(manifestResponse.status, 200);
const manifest = await manifestResponse.json();
assert.equal(manifest.version, '0.2.0');
for (const value of Object.values(manifest.platforms)) {
  assert.ok(metadata.some(a => a.url === value.url && a.signature === value.signature));
}
await mkdir('artifacts/0.2.0', { recursive: true });
await writeFile('artifacts/0.2.0/published-verification.json', JSON.stringify({ checked: new Date().toISOString(), results, redirect: redirect.status, updaterVersion: manifest.version }, null, 2) + '\n');
console.log('PASS production version, download URLs, checksums, updater and www redirect');
