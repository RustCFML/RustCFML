# Contributing to RustCFML

Contributions are welcome. This page explains what gets a pull request merged quickly.

## Before you start

- **New here?** Please **[open an Issue](https://github.com/RustCFML/RustCFML/issues)** before your first PR. Include a minimal CFML snippet that reproduces the problem, and the expected and actual behaviour.
- **Lucee is the compatibility reference.** RustCFML targets [cfdocs.org](https://cfdocs.org), with Lucee as the primary implementation. Check what Lucee actually does before changing behaviour. On rare occasions, where Lucee allows something genuinely unreasonable, we may choose not to match it.

## Keep each pull request to one feature or fix

A PR should do **one thing**: one bug fix, one feature, or one refactor. If you find a second problem while working, open a separate PR (or an Issue) for it.

This matters more here than on most projects:

- **Development moves fast.** `main` changes many times a day. A broad PR touches more files, so it conflicts sooner and goes stale before it can be merged. A small, focused PR can usually be reviewed and merged the same day.
- **Broad PRs are hard to review.** Changes with different risk, such as runtime code alongside release infrastructure, need different reviewers and different checks. Mixed together, nothing in the PR can be merged until everything is right, so one blocker holds up the rest.
- **Focused PRs are easy to revert.** If a change causes a regression, it can be backed out without losing unrelated work.

As a rule of thumb, if your PR description needs "and also", it is probably two PRs.

## Every change comes with a test

- **Start with a CFML test** that demonstrates the behaviour. It should fail before your change and pass after it. See **[Testing](docs/testing.md)** for how to write one and register it in `tests/runner.cfm`.
- **The test must pass on Lucee too.** The same suite runs against both engines, and a green run on both is the compatibility bar. A test that is RustCFML-only by design is marked in the runner with `rustcfmlOnly="true"`. [Testing](docs/testing.md#lucee-is-the-compatibility-reference) explains when that is appropriate.
- A PR that is only a failing test, demonstrating a bug for us to fix, is welcome too.

## Before you open the PR

Run these locally. CI runs the same gates, and a PR with a red check will not be merged:

```bash
cargo build --release
cargo test --workspace
cargo run --release -- tests/runner.cfm
```

Also:

- **Feature-gated code:** build with that feature enabled (e.g. `cargo build -p cfml-stdlib --features mssql_db`). The default build does not compile it.
- **Changed a shared type** (`CfmlValue`, `CfmlArray`, `CfmlStruct`…)? Also run `cargo build -p cfml-worker -p rustcfml-wasm --target wasm32-unknown-unknown`.
- **Added or changed a dependency?** Only add a dependency the PR's own code uses. Regenerate the licence notices with `./scripts/gen-licenses.sh`; CI fails if `THIRD-PARTY.txt` is stale.
- **Don't include unrelated changes:** reformatting, comment removal, or dependency and GitHub Action version bumps that the change does not need.

## Commit messages

Use a short, descriptive first line summarising the change, for example:

```
Add getProfileString, setProfileString, getProfileSections
Fix structSort numeric comparison on mixed keys
```
