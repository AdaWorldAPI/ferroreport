# Releasing

1. Update `version` in `Cargo.toml` (workspace), `crates/ferroreport-cli/Cargo.toml` (the
   `ferroreport` dependency) and `bindings/wasm/package.json`.
2. Commit, then tag and push: `git tag v0.7.0 && git push origin v0.7.0`.

The [release workflow](.github/workflows/release.yml) then publishes:

| Where | What | Setup |
|---|---|---|
| GitHub Releases | `ferroreport` CLI for Linux (x86_64, aarch64, static), macOS (x86_64, arm64), Windows | none |
| GitHub Container Registry | `ghcr.io/cristianencarnacion/ferroreport` (HTTP API, amd64 + arm64) | none (make the package public once in its settings) |
| npm | `ferroreport` (WebAssembly: Node.js, Deno, Bun, browsers) | variable `PUBLISH_NPM=true` |
| PyPI | `ferroreport` (wheels for Linux, macOS, Windows; Python 3.9+) | variable `PUBLISH_PYPI=true` |
| crates.io | `ferroreport` (library) and `ferroreport-cli` | variable `PUBLISH_CRATES=true` |

Repository variables live in *Settings → Secrets and variables → Actions → Variables*.

## One-time registry setup

All registries use **trusted publishing**: GitHub Actions proves its identity to the registry,
so no long-lived token is stored anywhere. Each registry needs the repository
`cristianEncarnacion/ferroreport`, the workflow `release.yml` and the environment shown below.

Current state:

| Registry | Trusted publisher | Notes |
|---|---|---|
| crates.io | configured for `ferroreport` and `ferroreport-cli` (environment `crates-io`) | the `CARGO_REGISTRY_TOKEN` secret is only a fallback and can be deleted |
| npm | to configure: package settings → Trusted publisher, workflow `release.yml`, environment `npm` | until then releases use `NPM_TOKEN`, which expires on 2026-11-28 |
| PyPI | to configure: pending publisher for project `ferroreport`, environment `pypi` | needs two-factor authentication on the PyPI account first; then set the repository variable `PUBLISH_PYPI` to `true` |

How each one was set up, for reference:

- **PyPI** (works before the first release): on pypi.org, *Your account → Publishing → Add a new
  pending publisher*: project `ferroreport`, owner `cristianEncarnacion`, repository
  `ferroreport`, workflow `release.yml`, environment `pypi`.
- **npm**: the first version must be published with a token, because npm can only trust a
  package that exists. Create a granular access token with publish rights on npmjs.com, save it
  as the repository secret `NPM_TOKEN`, release, then in the package settings add the trusted
  publisher (workflow `release.yml`, environment `npm`) and delete the secret.
- **crates.io**: same as npm. Create an API token (scope `publish-new`, `publish-update`) on
  crates.io, save it as the secret `CARGO_REGISTRY_TOKEN`, release, then add the trusted
  publisher for both crates (workflow `release.yml`, environment `crates-io`). The workflow
  prefers trusted publishing and only falls back to the secret.
