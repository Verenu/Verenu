# Verenu triage (experimental)

`verenu triage` hands a misbehaving Verenu install to your own coding agent. It mirrors `t3 triage` from T3 Code.

The command:

1. Runs read-only checks on the data directory and writes the results, with machine facts, to `context.md`.
2. Writes the Verenu playbook, [`src/PLAYBOOK.md`](src/PLAYBOOK.md), and the context pointer to `prompt.md`. Both go into a new folder under `<data directory>/triage/`.
3. Starts Claude Code or Codex in that folder with the playbook. The agent asks what went wrong, investigates, and drafts an issue.

If neither agent is installed, the command writes both files and prints their paths. Paste `prompt.md` into any coding agent.

## Run it

From the repository:

```bash
node packages/verenu-cli/bin/verenu.js triage
```

Use `node` rather than `npx`. `npx --prefix` runs a copy of the package cached by npm, which can be older than the source in this checkout. Triage then writes files from the old version.

Options:

- `--agent claude|codex` picks the agent. Without it, the command asks when both are installed.
- `--model MODEL` passes a model to the agent CLI.
- `--data-dir PATH` selects the data directory. The default matches the app: `%APPDATA%\Verenu` on Windows, `~/Library/Application Support/Verenu` on macOS, and `$XDG_CONFIG_HOME/Verenu` or `~/.config/Verenu` on Linux. `VERENU_APP_DATA_DIR_OVERRIDE` is also honored.
- `-h, --help` and `-v, --version` print help or the version and exit without running checks.

## What the command does not do

- It makes no network request itself. The agent may read from GitHub without asking: search issues, fetch a newer playbook, and clone the source. Posting an issue or comment needs your yes for that specific text.
- It writes only into its own `triage/` folder under the data directory. It never changes settings, the database, logs, or models. Only the agent can change those, and only with your approval for each step.
- It does not read the secure storage values. It checks that the Secret Service probe responds on Linux.

## Playbook and issue template

The playbook is plain Markdown. Edit `src/PLAYBOOK.md` to change what the agent does. Issues the agent drafts follow [`.github/ISSUE_TEMPLATE/via-triage.yml`](../../.github/ISSUE_TEMPLATE/via-triage.yml) and carry the `via-triage` label. Create that label in the repository before the first issue is filed.

## Tests

```bash
npm test --prefix packages/verenu-cli
```

The tests cover every check with a passing and a failing fixture, the context and prompt contents, agent selection, and the CLI flags. They use a fake agent CLI and never start a real one.

## Limits

- The checks read the data directory only. They cannot see the running app's state, such as its in-memory settings or whether a dictation is active.
- The Terms check reads `terms-consent.json`. The app does not write that file yet, so the check always reports unknown acceptance.
- Android app-private storage is not reachable from this desktop command.
- Windows is untested. The agent launch uses a shell on Windows, and that path has not been exercised.
