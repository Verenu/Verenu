# Verenu triage (experimental)

From the repository run `npx --prefix packages/verenu-cli verenu triage`, or run `npx . triage` from this directory. No npm publication is needed. Node.js 20 or newer and the `sqlite3` command are recommended.

Use `--data-dir PATH` or `VERENU_DATA_DIR` to select a data directory. `--json` emits a machine-readable report. `--yes` approves the available cache and invalid-settings fixes without prompting. Each fix backs up its inputs to `triage-backups`. Neither fix deletes transcript data. Close Verenu before applying fixes.

`--bundle` writes `verenu-triage-bundle.json` in the current directory without overwriting an existing file. It contains only check identifiers and statuses, never logs or settings. Inspect it before sharing.

Network access is disabled unless `terms-consent.json` in the data directory contains `{"accepted":true}`, or `--check-network` explicitly opts in. This file is a provisional integration contract; installations without it report unknown consent. Set `VERENU_CONVEX_URL` to an HTTPS endpoint to check its origin. No credentials are sent.

The executable check searches PATH; set `VERENU_BINARY` to the installed binary when it is elsewhere. Linux credential checks require `secret-tool` and `dbus-send`. Other platforms report secure storage as unsupported. Partial model files are detected, but checksums and process ownership are not yet verified. Android app-private storage is not accessible to this desktop command. SQLite verification requires the `sqlite3` executable.
