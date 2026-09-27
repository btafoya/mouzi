/**
 * Turns a folder chosen in the rule destination picker into the value the rule
 * should store.
 *
 * Rule destinations are resolved relative to the watched folder (see
 * `execute_rule` in `src-tauri/src/rules.rs`), so a folder that lives inside a
 * watched folder is stored relative to it and only folders outside every
 * watched folder keep an absolute path.
 */

const SEPARATORS = /[\\/]+/;

function toComponents(path: string): string[] {
  const parts = path.split(SEPARATORS).filter((part) => part.length > 0);
  // A leading separator is the root of an absolute path. Keep it as its own
  // component so the filesystem root itself ("/") can be a watched folder.
  return /^[\\/]/.test(path) ? ["/", ...parts] : parts;
}

/**
 * Windows paths are case-insensitive. The renderer has no direct OS signal
 * here, so the path shape decides: a backslash or a drive letter means Windows.
 * Either side of the comparison is enough, because a watched folder may have
 * been typed with forward slashes even on Windows.
 */
function isWindowsStyle(path: string): boolean {
  return path.includes("\\") || /^[a-z]:/i.test(path);
}

function sameComponent(a: string, b: string, caseInsensitive: boolean): boolean {
  return caseInsensitive ? a.toLowerCase() === b.toLowerCase() : a === b;
}

/**
 * Returns `selected` rewritten relative to the deepest watched folder that
 * contains it, `"."` when it is a watched folder itself, or `selected`
 * unchanged when it sits outside all of them.
 *
 * Every watched folder is considered, not just the one the rule is bound to:
 * rules created from the defaults carry `folder_id` `0`, which matches no
 * folder row, so keying off the rule's own folder would leave those
 * destinations absolute.
 */
export function relativizeDestination(
  selected: string,
  watchedFolders: string[],
): string {
  const selectedParts = toComponents(selected);
  if (selectedParts.length === 0) return selected;

  let deepest = -1;
  let relative: string | null = null;

  for (const folder of watchedFolders) {
    const folderParts = toComponents(folder);
    if (folderParts.length === 0) continue;
    if (folderParts.length > selectedParts.length) continue;
    if (folderParts.length <= deepest) continue;

    const caseInsensitive = isWindowsStyle(folder) || isWindowsStyle(selected);
    const contained = folderParts.every((part, i) =>
      sameComponent(part, selectedParts[i], caseInsensitive),
    );
    if (!contained) continue;

    deepest = folderParts.length;
    relative = selectedParts.slice(folderParts.length).join("/") || ".";
  }

  return relative ?? selected;
}

/**
 * Maps a folder-picker result to the destination a rule should store, or `null`
 * when the picker was dismissed and the current value must be left alone.
 */
export function nextDestination(
  selected: string | string[] | null,
  watchedFolders: string[],
): string | null {
  const path = Array.isArray(selected) ? selected[0] : selected;
  if (!path) return null;
  return relativizeDestination(path, watchedFolders);
}
