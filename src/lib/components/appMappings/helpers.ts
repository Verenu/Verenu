import { cleanAppName, normalizeExe, type InstalledApp } from '../../appMappings';

export function customExeFromSearch(search: string): string {
  return normalizeExe(search).replace(/\.exe$/, '') + '.exe';
}

export function matchesAppSearch(app: InstalledApp, search: string) {
  const query = search.trim().toLowerCase();
  if (!query) return true;

  const appName = cleanAppName(app.name || app.exe).toLowerCase();
  const appExe = normalizeExe(app.exe);
  const compactQuery = query.replace(/[^a-z0-9]/g, '');
  const compactName = appName.replace(/[^a-z0-9]/g, '');
  const compactExe = appExe.replace(/[^a-z0-9]/g, '');

  return appName.includes(query)
    || appExe.includes(query)
    || (compactQuery.length > 0 && (compactName.includes(compactQuery) || compactExe.includes(compactQuery)));
}

/**
 * Apps matching `search`, best first: names that start with the query, then
 * names with a word that starts with it, then any other match. Ties keep the
 * input order (the platform lists apps alphabetically). An empty query keeps
 * the list as is.
 */
export function rankAppMatches(apps: InstalledApp[], search: string): InstalledApp[] {
  const query = search.trim().toLowerCase();
  if (!query) return apps;
  const score = (app: InstalledApp) => {
    const name = cleanAppName(app.name || app.exe).toLowerCase();
    if (name.startsWith(query)) return 0;
    if (name.split(/[^a-z0-9]+/).some((word) => word.startsWith(query))) return 1;
    return 2;
  };
  return apps
    .filter((app) => matchesAppSearch(app, search))
    .map((app, index) => ({ app, index, rank: score(app) }))
    .sort((a, b) => a.rank - b.rank || a.index - b.index)
    .map((entry) => entry.app);
}
