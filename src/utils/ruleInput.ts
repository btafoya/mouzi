export function normalizeExtensions(entries: string[]): string[] {
  return [...new Set(entries.flatMap(entry => entry.split(',')).map(entry => entry.trim().replace(/^\.+/, '').toLowerCase()).filter(Boolean))];
}
