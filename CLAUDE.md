# Maya

Building, testing, the layout and the release workflow are in
[docs/DEVELOPING.md](docs/DEVELOPING.md).

## Pull requests bump the version

Every pull request that changes what ships bumps the version, in the same
pull request, so that merging it publishes a release. Merging to `main`
publishes `v<version>` whenever no release with that version exists yet;
a pull request that leaves the version alone releases nothing.

- Bump from the version on `main` (it may have moved since the branch was
  made), following semver and the pull request's commits:
  - a breaking change: the major version (minor before 1.0);
  - any `feat`: the minor version;
  - only `fix` or `perf`: the patch version;
  - only `docs`, `ci`, `test` or `chore`: no bump, since nothing ships.
- Set it with `sh scripts/set-version.sh <major.minor.patch>`, which keeps
  `src-tauri/tauri.conf.json`, `package.json`, `Cargo.toml` and
  `Cargo.lock` in step (CI fails when they disagree), and commit it on its
  own as `chore(release): <version>`.
- Bump once per pull request. If `main` moves on before the merge, redo
  the bump against `main`'s new version.
