# ProjectorHUD

Cross-platform Rust (egui) app to monitor and control networked projectors. See README.md.

- `cargo test` runs the drivers and workers against the built-in simulator; keep it passing.
- Before pushing: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` (CI fails on any warning).
- `cargo run --release` runs the app; `-- --sim` runs only the simulated projectors.
- Never send control commands to real projectors on the network without being asked.

## CI and releases

Set up the same way as the NewQ repository.

- CI (`.github/workflows/ci.yml`) runs fmt, clippy and the tests on Ubuntu, Windows and macOS,
  once per change: on pull requests and on `main` after a merge. A rebase merge of a pull
  request that CI passed on is not built again (the `gate` job). Changes that only touch
  `*.md` or `LICENSE` skip CI. A branch has no CI until it has a pull request.
- Work goes through pull requests, merged with **rebase** so `main` stays linear.
- **Releasing:** follow `.claude/skills/release/SKILL.md`. In short: bump `version` in
  `Cargo.toml` in a PR, merge, wait for CI on `main`, then run the `Cut release` workflow
  (dry run first) with the tag `vX.Y.Z`. It checks the tag against `Cargo.toml`, tags `main`
  and starts the `Release` workflow, which builds the downloads with
  `scripts/package_release.sh` and publishes them.

## Identity for commits

Claude's own accounts, set up by the repo owner: email `r2.assistant.42@gmail.com`, GitHub `r2a42`.
Before committing, if `git config user.email` is unset or a machine hostname, run:

```
git config user.name "r2a42"
git config user.email "r2.assistant.42@gmail.com"
```

Never author commits with a hostname address or `noreply@anthropic.com`.
