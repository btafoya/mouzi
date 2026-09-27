import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import i18next from 'i18next';

const directory = new URL('../src/i18n/locales/', import.meta.url);
const locales = Object.fromEntries(await Promise.all(
  (await readdir(directory)).filter(file => file.endsWith('.json')).map(async file => [
    file.slice(0, -5), JSON.parse(await readFile(new URL(file, directory), 'utf8')),
  ]),
));
function strings(object, prefix = '') {
  return Object.fromEntries(Object.entries(object).flatMap(([key, value]) => {
    if (!prefix && key === '__translator_info__') return [];
    const path = prefix ? `${prefix}.${key}` : key;
    return typeof value === 'string' ? [[path, value]] : Object.entries(strings(value, path));
  }));
}
const tokens = value => [...value.matchAll(/\{\{[^{}]+\}\}|\{[^{}]+\}/g)].map(match => match[0]).sort();
const reference = strings(locales.en);

for (const [language, locale] of Object.entries(locales)) {
  test(`${language}: all UI keys and placeholders are present without fallback (#67)`, async () => {
    assert.ok(locale.beta, 'Missing organization translations');
    const translated = strings(locale);
    assert.deepEqual(Object.keys(translated).sort(), Object.keys(reference).sort());
    for (const [key, source] of Object.entries(reference)) {
      assert.ok(translated[key].trim(), `${language}: empty ${key}`);
      assert.deepEqual(tokens(translated[key]), tokens(source), `${language}: placeholders in ${key}`);
    }
    // Disable fallback so a missing runtime lookup cannot silently use English.
    const instance = i18next.createInstance();
    await instance.init({ lng: language, fallbackLng: false, resources: { [language]: { translation: locale } } });
    for (const key of Object.keys(reference)) {
      const fullKey = key;
      assert.equal(instance.exists(fullKey), true, fullKey);
      assert.equal(instance.t(fullKey, { count: 3 }), translated[key].replaceAll('{{count}}', '3'));
    }
  });
}
