# Releasing

Two channels, one branch. `main` is the bleeding edge; stability is a property of
a **release**, not of a branch, so nothing is ever rebuilt or merged to promote
it — the artifacts that were tested are the artifacts that ship.

| | Edge | Stable |
|---|---|---|
| What | every version tag | an edge build promoted after it has been run in anger |
| GitHub | prerelease | the release `/releases/latest` resolves to |
| Who it is for | trying a fix, following development | production |

## Cutting an edge build

1. Land the change on `main` with the [verification gate](../CLAUDE.md) green.
2. Bump the version in `Cargo.toml`, build, commit, tag, push:

```bash
sed -i '' 's/0\.653\.13/0.653.14/g' Cargo.toml
cargo build --release            # refreshes Cargo.lock
git commit -am "v0.653.14: <what changed>"
git tag v0.653.14
git push origin main
git push origin v0.653.14
```

> ⚠️ **Push tags ONE AT A TIME.** GitHub triggers no workflow at all when more
> than three tags arrive in a single push, silently — v0.653.4 through v0.653.9
> have no binaries for exactly this reason, and nothing reported an error.

The `Release Binaries` workflow then runs the licence gate, builds all four
platforms with PGO, and publishes a **prerelease** with the binaries and the
licence notices attached.

## Promoting to stable

When a build has proved itself — booted the real applications, served traffic,
survived a day — promote it:

```bash
gh workflow run promote.yml -f tag=v0.653.14
```

or run *Promote Release to Stable* from the Actions tab. It checks the release
exists and carries all four binaries (a release whose build failed must never
become the download everyone gets), then clears the prerelease flag and marks it
latest. `/releases/latest`, which every download link in the README and docs
points at, now resolves to it.

Demote just as easily if something surfaces later:

```bash
gh release edit v0.653.14 --prerelease=true
gh release edit v0.653.13 --latest      # fall back to the previous stable
```

## Why not git-flow / twgit

Branch-per-channel earns its keep when several people land work concurrently, or
when a fix must go onto a shipped version while `main` has moved on. Neither is
true here yet, and until it is, a `develop` branch costs a merge per change and
returns nothing that the prerelease flag does not.

The moment to revisit is the **first backport**: a stable line that needs a fix
`main` has already moved past. That is when `release/0.653` starts paying for
itself, and it can be branched from the tag at that point without any of this
changing.

## What not to do

- **Do not move a tag.** A tag that changes breaks every checksum, cache and pin
  that already saw it. If a tagged build is wrong, cut the next patch version;
  the cost is one number.
- **Do not promote a release without assets.** The promote workflow refuses, but
  the reason matters: a failed licence gate skips the build and release jobs
  entirely, leaving a tag with no binaries and no obvious sign of it.
- **Do not regenerate `THIRD-PARTY.txt` casually.** Run
  `./scripts/gen-licenses.sh` after a *dependency* change and commit it —
  cargo-about must be the pinned 0.9.1 or the diff is noise.
