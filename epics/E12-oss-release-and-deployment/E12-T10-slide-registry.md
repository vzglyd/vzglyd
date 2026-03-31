# E12-T10: Slide Registry Index

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P2 (medium) |
| **Estimate** | S |
| **Blocked by** | E12-T9 |
| **Blocks** | E12-T11 |

## Description

Create a `vzglyd/registry` GitHub repository containing a curated `index.json` file that lists all known VZGLYD slides with their metadata and release URLs. This is the discovery layer: users can browse what exists, and the `vzglyd-get` tool (E12-T11) will query it programmatically. The registry is intentionally simple — a static JSON file in a git repo — and can evolve into something more sophisticated later.

## Background

Without a registry, the answer to "what slides exist for VZGLYD?" is "search GitHub for `vzglyd slide`." That's fine for day one but terrible for adoption. A curated list signals that the project is stewarded and makes first-time users immediately productive.

The simplest possible registry is a JSON file at a well-known URL. No database, no server, no auth. GitHub's raw content CDN serves it instantly and the PR model governs additions.

## Registry repository structure

```
vzglyd/registry/
├── README.md            # What this is, how to submit a slide
├── index.json           # The registry
├── CONTRIBUTING.md      # How to add your slide
└── .github/
    └── workflows/
        └── validate.yml # Validates index.json on every PR
```

## index.json schema

```json
{
  "schema_version": 1,
  "updated_at": "2026-03-30T00:00:00Z",
  "slides": [
    {
      "name": "weather",
      "display_name": "Weather Forecast",
      "description": "Australian Bureau of Meteorology 3-day forecast with temperature and condition icons",
      "author": "vzglyd",
      "repo": "https://github.com/vzglyd/slide-weather",
      "tags": ["weather", "api", "australia"],
      "requires_config": true,
      "config_keys": ["location_id"],
      "abi_version": 1,
      "releases": [
        {
          "version": "0.2.0",
          "date": "2026-03-30",
          "url": "https://github.com/vzglyd/slide-weather/releases/download/v0.2.0/weather-0.2.0.vzglyd",
          "sha256": "abc123..."
        }
      ]
    },
    {
      "name": "clock",
      "display_name": "Clock",
      "description": "Minimal analogue and digital clock",
      "author": "vzglyd",
      "repo": "https://github.com/vzglyd/slide-clock",
      "tags": ["clock", "time", "no-sidecar"],
      "requires_config": false,
      "config_keys": [],
      "abi_version": 1,
      "releases": [
        {
          "version": "0.1.0",
          "date": "2026-03-01",
          "url": "https://github.com/vzglyd/slide-clock/releases/download/v0.1.0/clock-0.1.0.vzglyd",
          "sha256": "def456..."
        }
      ]
    }
  ]
}
```

### Schema fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Machine name, matches `manifest.json` name |
| `display_name` | string | yes | Human-readable name |
| `description` | string | yes | One sentence description |
| `author` | string | yes | GitHub username or org |
| `repo` | string | yes | GitHub repo URL |
| `tags` | string[] | yes | Searchable tags |
| `requires_config` | bool | yes | Whether the slide needs a config file |
| `config_keys` | string[] | yes | Config file keys required (empty if none) |
| `abi_version` | int | yes | ABI version the slide was built against |
| `releases[].version` | string | yes | semver |
| `releases[].date` | string | yes | ISO 8601 date |
| `releases[].url` | string | yes | Direct download URL for the .vzglyd file |
| `releases[].sha256` | string | yes | Hex SHA-256 of the .vzglyd file |

## Validation workflow

```yaml
# .github/workflows/validate.yml
name: Validate Registry

on:
  pull_request:
  push:
    branches: [main]

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Validate JSON schema
        run: |
          # jq exits non-zero if JSON is malformed
          jq '.' index.json > /dev/null
          # Check required fields on each slide
          jq -e '
            .slides[] |
            .name and .display_name and .description and
            .author and .repo and .abi_version
          ' index.json > /dev/null
      - name: Check all release URLs are reachable
        run: |
          jq -r '.slides[].releases[].url' index.json | \
          while read url; do
            echo "Checking: $url"
            curl -fsSL --head "$url" > /dev/null || {
              echo "FAIL: $url is not reachable"
              exit 1
            }
          done
      - name: Verify SHA-256 checksums
        run: |
          # Download each release and verify its checksum
          # (Only run on pushes to main, not PRs, to avoid bandwidth waste)
          if [[ "${{ github.event_name }}" == "push" ]]; then
            jq -r '.slides[].releases[] | "\(.sha256)  \(.url)"' index.json | \
            while IFS='  ' read -r sha url; do
              echo "Verifying: $url"
              actual=$(curl -fsSL "$url" | sha256sum | awk '{print $1}')
              [[ "$actual" == "$sha" ]] || {
                echo "FAIL: checksum mismatch for $url"
                echo "  expected: $sha"
                echo "  actual:   $actual"
                exit 1
              }
            done
          fi
```

## Community submission process

`registry/CONTRIBUTING.md` documents how to add a community slide:

1. Build and publish your slide to a public GitHub repo with a tagged release
2. Fork `vzglyd/registry`
3. Add your slide's entry to `index.json` (follow the schema)
4. Open a PR — CI validates the entry, a maintainer reviews and merges

Requirements for community submissions:
- Repo must be public
- Must have a tagged release with a `.vzglyd` artifact
- Must have a README
- Must have a LICENSE

The maintainer review is lightweight: check the entry is well-formed, the repo exists, and the `.vzglyd` is genuine. No code review of the slide itself — slides run in WASM sandboxes.

## Raw URL for vzglyd-get

The `vzglyd-get` tool (E12-T11) fetches from:

```
https://raw.githubusercontent.com/vzglyd/registry/main/index.json
```

This URL is stable and CDN-cached by GitHub. Updating the registry is a git commit to main.

## Acceptance criteria

- [ ] `vzglyd/registry` repo exists
- [ ] `index.json` contains all official `vzglyd/slide-*` slides
- [ ] All entries have valid URLs pointing to real .vzglyd release artifacts
- [ ] SHA-256 checksums are present and correct for all releases
- [ ] Validation CI passes on the initial commit
- [ ] `CONTRIBUTING.md` documents community submission process
- [ ] At least one community-contributed slide is accepted as a proof-of-concept (can be a trivial test slide)

## Files to create (in vzglyd/registry)

| File | Purpose |
|------|---------|
| `index.json` | The registry |
| `README.md` | What the registry is and how to use it |
| `CONTRIBUTING.md` | Community submission guide |
| `.github/workflows/validate.yml` | Registry validation CI |
