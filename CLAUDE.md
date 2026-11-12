# ProjectorHUD

Cross-platform Rust (egui) app to monitor and control networked projectors. See README.md.

- `cargo test` runs the drivers and workers against the built-in simulator; keep it passing.
- `cargo run --release` runs the app; `-- --sim` runs only the simulated projectors.
- Never send control commands to real projectors on the network without being asked.

## Identity for commits

Claude's own accounts, set up by the repo owner: email `r2.assistant.42@gmail.com`, GitHub `r2a42`.
Before committing, if `git config user.email` is unset or a machine hostname, run:

```
git config user.name "r2a42"
git config user.email "r2.assistant.42@gmail.com"
```

Never author commits with a hostname address or `noreply@anthropic.com`.
