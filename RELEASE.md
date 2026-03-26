# Release Guide

This project uses **Release Please** for automated semantic versioning and changelog generation based on **Conventional Commits**.

## How It Works

1. **Every push to `main`** triggers the `release-please` workflow, which analyzes commits since the last release and creates or updates a release PR.

2. **The release PR** bumps the version in `Cargo.toml`, updates `CHANGELOG.md`, and stays open collecting further changes until you're ready to ship.

3. **Merging the release PR** triggers the actual release: Release Please creates the git tag and GitHub release, then the build jobs produce binaries for all three platforms and attach them.

## Version Bump Rules

| Commit type | Version impact |
|---|---|
| `feat:` | Minor bump (`0.1.x` → `0.2.0`) |
| `fix:` | Patch bump (`0.1.0` → `0.1.1`) |
| `feat!:`, `fix!:`, or any type with `BREAKING CHANGE:` footer | Major bump (`0.1.0` → `1.0.0`) |
| `refactor:`, `perf:`, `chore:`, `docs:`, `style:`, `test:`, `ci:` | No bump (still appears in changelog) |

> Before `v1.0.0`, breaking changes bump **minor** instead of major (configured via `bump-minor-pre-major: true` in `release-please-config.json`).

## Commit Message Format

```
<type>(<optional scope>): <description>

# Examples
feat: add Azure provider
fix(network): resolve firewall merge bug
feat!: redesign provider API
docs: improve README
```

The `commit-msg` git hook in `.githooks/` validates this locally. Run `git config core.hooksPath .githooks` once after cloning to activate it. PR commit messages are also validated by the `conventional-commits` workflow.

## Workflow Files

| File | Purpose |
|---|---|
| `.github/workflows/ci.yml` | fmt, clippy, tests on every push and PR |
| `.github/workflows/conventional-commits.yml` | Validates commit messages on PRs |
| `.github/workflows/release-please.yml` | Creates release PRs and builds/publishes binaries |

## Manual Trigger

The release workflow can also be triggered manually from the **Actions** tab (`workflow_dispatch`) if needed.

