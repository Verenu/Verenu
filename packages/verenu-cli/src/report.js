export function report(results) { return results.map(({status, name, message}) => `${status.toUpperCase()} ${name}: ${message}`).join('\n'); }
