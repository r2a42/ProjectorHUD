---
name: release
description: Cut a ProjectorHUD release end to end (bump, PR, CI, merge, main CI, dry run, real run, verify). Use whenever the user asks to release, ship or publish a version.
---

# Releasing ProjectorHUD

The same process as the NewQ repository. The user asks once. **Do not hand control back
until the release is published and verified, or you are blocked on something only they
can do.**

## Steps

1. **Bump.** `version` in `Cargo.toml` and the `projectorhud` entry in `Cargo.lock`
   (`cargo check` updates it). Commit "Bump version to X.Y.Z". Run `cargo fmt --check`,
   `cargo clippy --all-targets -- -D warnings` and `cargo test` first: one clean push
   beats three.
2. **PR.** Push a branch and open a pull request against `main`.
3. **CI.** Three jobs (ubuntu, windows, macos) plus `gate`. On a failure, read the job
   log and decide:
   - Ours (fmt, clippy or a test failed): fix, run the checks, push. The push restarts CI.
   - Infrastructure (apt 5xx or timeout before any test ran, a runner that never
     started): re-run the failed jobs once; a second failure is real.
4. **Merge.** When all checks are green on the head, merge with **rebase** (never merge
   or squash: `main` stays linear): `gh pr merge N --rebase --delete-branch`.
5. **Main CI.** The rebase merge has the same tree, so the `gate` job skips the builds and
   the run is quick. It must be `completed`/`success`.
6. **Dry run.** `gh workflow run cut-release.yml --ref main -f tag=vX.Y.Z -f dry_run=true`,
   then wait for `success` (about 15 seconds).
7. **Real run.** The same with `dry_run=false`. It tags `main` and starts the `Release`
   workflow (builds for all three platforms, publishes a draft, then releases it).
8. **Verify.** The `Release` run must be `success`, then `gh release view vX.Y.Z` must
   show the Windows, macOS and Linux downloads and `SHA256SUMS.txt`. Report the link and
   what is in it. A `Release` failure: read the job log, fix or report, do not stop silently.
9. After the merge, restart the local branch from `main`: a rebase merge gives the commits
   new ids, so the old branch holds old copies.

`Cut release` is only accepted from the repository owner's account (`r2a42`). It can also be
started by the owner commenting `/release vX.Y.Z` (or `/release vX.Y.Z dry-run`) on any
issue or pull request.

## Waiting

`scripts/gh_wait.sh` polls the public GitHub API once a minute with plain `curl`:

```
scripts/gh_wait.sh pr 3               # all checks on the PR head finished (exit 0 = all passed)
scripts/gh_wait.sh latest ci.yml      # id/status/sha of the newest run
scripts/gh_wait.sh run 40796485911    # a workflow run finished (exit 0 = success)
scripts/gh_wait.sh release v0.1.0     # the release exists; prints its assets
```

Where the `gh` CLI is signed in, `gh pr checks N --watch` and `gh run watch ID --exit-status`
do the same.

## Signing

`release.yml` signs and notarises the Mac app when the repository has the Apple secrets
(`MACOS_CERT_P12`, `MACOS_CERT_PASSWORD`, `APPLE_API_KEY_P8`, `APPLE_API_KEY_ID`,
`APPLE_API_ISSUER_ID`: the same five NewQ uses), and signs the Windows program with
`WINDOWS_CERT_PFX` / `WINDOWS_CERT_PASSWORD`. Without them the Mac app is ad-hoc signed and
the Windows program unsigned; when the secrets are added, update `.github/release_notes.md`,
which tells users the builds are unsigned.
