# Releasing r2kit

This checklist is for maintainers. It keeps the crate, Git tag, changelog, and
GitHub release tied to the same reviewed commit.

## Prepare the release commit

1. Update the version in `Cargo.toml` and run `cargo check` so `Cargo.lock`
   records the same package version.
2. Move the relevant entries from `[Unreleased]` into a dated release section
   in `CHANGELOG.md` and update its comparison links.
3. For the first crates.io release, replace the README's Git installation
   examples and pre-release notice with the crates.io installation command.
4. Run `./scripts/release-check.sh`. It requires a clean worktree, runs the
   complete offline quality suite, and verifies the exact package that Cargo
   would upload.
5. Commit the release changes, push them to `main`, and wait for every required
   CI job to pass.

Live tests are deliberately separate because they need bucket-scoped secrets.
Before a release that changes R2 behavior, run the contract and stress commands
documented in `CONTRIBUTING.md` against the dedicated test bucket.

## Publish

Publishing is intentionally not automated by the check script. A crates.io
version cannot be deleted after publication; it can only be yanked.

From the clean, reviewed release commit:

```sh
cargo login
./scripts/release-check.sh
git tag -a v0.1.0 -m "r2kit 0.1.0"
cargo publish --locked
git push origin v0.1.0
```

Create a GitHub release from that tag and copy the matching changelog section
into its notes. Confirm the version is visible on crates.io and its docs build
is available on docs.rs.

If `cargo publish` fails, fix the release commit and recreate the **local** tag.
Never push the tag before crates.io accepts the package.

## After publishing

Restore an empty `[Unreleased]` section if the next development commit needs
one, verify the README links, and start collecting changes for the next release.
