import { cleanAppName, normalizeExe, type InstalledApp } from '../../appMappings';
import { isAndroid } from '../../platform';

export function customExeFromSearch(search: string): string {
  return normalizeExe(search).replace(/\.exe$/, '') + '.exe';
}

/**
 * Whether an app matches a search. Android packages all start "com." and share
 * segments like "google.android", so only the visible name is searched there.
 */
export function matchesAppSearch(app: InstalledApp, search: string, nameOnly = isAndroid) {
  const query = search.trim().toLowerCase();
  return matchesSearch(app, query, query.replace(/[^a-z0-9]/g, ''), nameOnly);
}

function matchesName(appName: string, query: string, compactQuery: string) {
  if (appName.includes(query)) return true;
  const compactName = appName.replace(/[^a-z0-9]/g, '');
  return compactQuery.length > 0 && compactName.includes(compactQuery);
}

function matchesSearch(app: InstalledApp, query: string, compactQuery: string, nameOnly: boolean) {
  if (!query) return true;
  const appName = cleanAppName(app.name || app.exe).toLowerCase();
  if (matchesName(appName, query, compactQuery)) return true;
  if (nameOnly) return false;
  const appExe = normalizeExe(app.exe);
  return appExe.includes(query)
    || (compactQuery.length > 0 && appExe.replace(/[^a-z0-9]/g, '').includes(compactQuery));
}

/**
 * Apps matching `search`. Android only: best first — names that start with the query, then
 * names with a word that starts with it, then any other match. Ties keep the
 * input order (the platform lists apps alphabetically). An empty query keeps
 * the list as is.
 */
export function rankAppMatches(apps: InstalledApp[], search: string, android = isAndroid): InstalledApp[] {
  const query = search.trim().toLowerCase();
  if (!query) return apps;
  const compactQuery = query.replace(/[^a-z0-9]/g, '');
  // Desktop keeps its established behaviour: filter only, in the platform's order.
  if (!android) return apps.filter((app) => matchesSearch(app, query, compactQuery, false));
  const score = (name: string) => {
    if (name.startsWith(query)) return 0;
    if (name.split(/[^a-z0-9]+/).some((word) => word.startsWith(query))) return 1;
    return 2;
  };
  return apps
    .flatMap((app, index) => {
      const name = cleanAppName(app.name || app.exe).toLowerCase();
      return matchesName(name, query, compactQuery) ? [{ app, index, rank: score(name) }] : [];
    })
    .sort((a, b) => a.rank - b.rank || a.index - b.index)
    .map((entry) => entry.app);
}
