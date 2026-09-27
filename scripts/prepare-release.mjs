import assert from 'node:assert/strict';
import { readFile, writeFile, readdir, stat } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { createHash } from 'node:crypto';

const version = JSON.parse(await readFile('src-tauri/tauri.conf.json', 'utf8')).version;
assert.match(version, /^\d+\.\d+\.\d+$/);
const dir = resolve(process.argv[2] || `artifacts/${version}/release`);
const base = `https://github.com/hsr88/mouzi/releases/download/v${version}`;
const names = {
  nsis: `Mouzi_${version}_x64-setup.exe`,
  msi: `Mouzi_${version}_x64_en-US.msi`,
  portable: `Mouzi_${version}_x64-portable.exe`,
  appimage: `Mouzi_${version}_amd64.AppImage`,
  deb: `Mouzi_${version}_amd64.deb`,
  rpm: `Mouzi-${version}-1.x86_64.rpm`,
};
const metadata = [];
for (const [kind, file] of Object.entries(names)) {
  const bytes = await readFile(join(dir, file));
  const entry = { kind, file, bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex'), url: `${base}/${file}` };
  if (kind !== 'portable') {
    entry.signature = (await readFile(join(dir, `${file}.sig`), 'utf8')).trim();
    assert.ok(entry.signature.length > 80, `Missing signature: ${file}`);
  }
  metadata.push(entry);
}
const platforms = {};
for (const [platform, kind] of Object.entries({ 'windows-x86_64': 'nsis', 'windows-x86_64-nsis': 'nsis', 'windows-x86_64-msi': 'msi', 'linux-x86_64': 'appimage', 'linux-x86_64-appimage': 'appimage', 'linux-x86_64-deb': 'deb', 'linux-x86_64-rpm': 'rpm' })) {
  const entry = metadata.find(m => m.kind === kind);
  platforms[platform] = { signature: entry.signature, url: entry.url };
}
const notes = await readFile(`.github/release-notes/v${version}.md`, 'utf8');
await writeFile(join(dir, 'latest.json'), JSON.stringify({ version, notes, pub_date: new Date().toISOString(), platforms }, null, 2) + '\n');
const files = (await readdir(dir)).filter(f => f !== 'SHA256SUMS.txt').sort();
const checksums = [];
for (const file of files) {
  if (!(await stat(join(dir, file))).isFile()) continue;
  checksums.push(`${createHash('sha256').update(await readFile(join(dir, file))).digest('hex')}  ${file}`);
}
await writeFile(join(dir, 'SHA256SUMS.txt'), checksums.join('\n') + '\n');
await writeFile(resolve(`artifacts/${version}/artifact-metadata.json`), JSON.stringify(metadata, null, 2) + '\n');
console.log(JSON.stringify(metadata.map(({ signature, ...entry }) => entry), null, 2));
