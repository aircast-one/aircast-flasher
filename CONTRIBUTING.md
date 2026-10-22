# Contributing to Aircast Flasher

Aircast Flasher is a Tauri 2 desktop app: a React/TypeScript frontend driving
a Rust core (`src-tauri` plus the `flasher-core`, `flasher-ops`, and
`flasher-cli` crates in `crates/`). The Rust side owns privileged work
(device enumeration, `dd`, provisioning); the frontend is the wizard UI.

## Setup

- Node (see `package.json` for the toolchain), Rust stable, `cargo`.
- Install deps: `npm install`.
- Run the app: `just dev` (equivalent to `npm run tauri dev`).

`just` with no args lists every recipe, grouped by `dev`/`build`/`test`/`release`.

## Project shape

- `src/` — React frontend (wizard steps: OS → Network → Access → Storage →
  Write, plus the Devices sidebar).
- `src-tauri/` — Tauri shell, IPC commands.
- `crates/flasher-core` — flashing/provisioning logic shared by the app and CLI.
- `crates/flasher-ops` — release manifest / update-channel handling.
- `crates/flasher-cli` — `aircast-flasher-cli`, a standalone CLI + MCP server
  that wraps `flasher-core` for scripting and for Claude/agent use (`just cli`
  to build it).
- `e2e/` — WebdriverIO specs driven through the embedded Tauri driver.

Read the README before touching the wizard flow or the Tailscale/Headscale
integration — it documents *why* each step is split the way it is (hostname
collision handling, pre-auth-key vs. post-boot sign-in, the Devices view's
`/healthz` probing), not just what the code does.

## Before committing

Run what actually covers your change, not the full `check` target on every
commit:

```
just test   # cargo test --workspace + npm run test:unit + npm run test:scripts
just e2e    # real app, WebdriverIO through the embedded Tauri driver
just check  # everything CI runs (test + e2e) — run this before pushing
```

CI (`.github/workflows/ci.yml`, `linux.yml`, `windows.yml`) runs the same
`just check` across platforms plus platform-specific build jobs
(`desktop-release.yml` handles tagged releases). Match whatever CI runs
rather than guessing at a subset.

There are no pre-commit hooks in this repo — nothing to skip, nothing to
bypass. Don't add any that can be disabled with an env var or a flag.

## Versions

`package.json` and `Cargo.toml` (`workspace.package.version`) are kept in
sync manually on each release commit (`chore: bump version to X.Y.Z`) — bump
both together, never one alone.

## Releases

Releases are cut with `just release <patch|minor|major|dev|staging>`
(`scripts/release.sh`), never by hand-pushing a tag.

## Commits

Conventional-ish prefixes (`fix:`, `feat:`, `chore:`) with a scope in
parens (`fix(tang): ...`, `feat(cli): ...`) — look at `git log` for the
pattern used in a given area before picking a new one.
