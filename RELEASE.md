# Automated Semantic Release Guide

This project uses **cargo-release** for automated semantic versioning based on **conventional commits**.

## How It Works

1. **Conventional Commits**: All commits to `main` must follow the conventional commit format:
   - `feat: add new feature` → Bumps MINOR version (0.1.0 → 0.2.0)
   - `fix: bug fix` → Bumps PATCH version (0.1.0 → 0.1.1)
   - `feat!: breaking change` or `fix!: breaking change` → Bumps MAJOR version (0.1.0 → 1.0.0)
   - `chore:`, `docs:`, `style:`, `test:`, `refactor:`, `perf:` → No version bump

2. **Pull Requests**: The `conventional-commits` workflow on PRs validates that your commits follow the format.

3. **Automatic Release**:
   - When you push to `main`, the `automated-release` workflow runs
   - It validates tests, linting, and conventional commits
   - If conventional commits are detected, `cargo-release` automatically:
     - Bumps the version in `Cargo.toml` based on change type
     - Creates a git tag (e.g., `v0.2.0`)
     - Pushes the tag and commit to GitHub
   - The existing `release` workflow then triggers on the tag and builds binaries

## Manual Release (if needed)

If you want to manually trigger a release:

```bash
# Dry-run (shows what would happen)
cargo release

# Actually perform the release and push to GitHub
cargo release --execute
```

## Version Bump Rules

| Commit Type | Version Impact |
|------------|-------------------|
| `feat:` | Minor bump (0.1.x → 0.2.0) |
| `fix:` | Patch bump (0.1.0 → 0.1.1) |
| `feat!:` or `fix!:` | Major bump (0.1.0 → 1.0.0) |
| `refactor:`, `perf:`, `chore:`, `docs:`, `style:`, `test:` | No bump |

## Examples

```bash
# Feature release (0.1.0 → 0.2.0)
git commit -m "feat: add provider abstraction support"

# Bug fix (0.1.0 → 0.1.1)
git commit -m "fix: resolve firewall mapping issue"

# Breaking change (0.1.0 → 1.0.0)
git commit -m "feat!: redesign configuration format\n\nBREAKING CHANGE: config files must now use new format"

# No version bump
git commit -m "chore: update dependencies"
git commit -m "docs: improve README"
git commit -m "test: add integration tests"
```

## Workflow Files

- **`.github/workflows/ci.yml`**: Runs tests and linting on all PRs
- **`.github/workflows/conventional-commits.yml`**: Validates commit messages
- **`.github/workflows/automated-release.yml`**: Automatically creates releases on push to main
- **`.github/workflows/release.yml`**: Builds and publishes binaries when tags are pushed

## GitHub Token Permission

The release workflow uses `GITHUB_TOKEN` which is automatically available in GitHub Actions. No additional setup needed!

## Disable Auto-Release

If you want to manually control releases, comment out or remove the push in `.github/workflows/automated-release.yml`:

```yaml
- name: Create release
  if: steps.check-release.outputs.needs-release == 'true'
  run: cargo release --no-confirm --git-token ${{ secrets.GITHUB_TOKEN }}
```

Then manually run:
```bash
cargo release --execute
```
