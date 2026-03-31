# E12-T11: vzglyd-get Slide Installer CLI

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P3 (nice-to-have) |
| **Estimate** | M |
| **Blocked by** | E12-T10 |
| **Blocks** | - |

## Description

A small CLI tool, `vzglyd-get`, that queries the slide registry (E12-T10) and installs `.vzglyd` packages into the slides directory. Think `apt install` but for VZGLYD slides. The tool ships as a subcommand of the main `vzglyd` binary (`vzglyd get install weather`) or as a standalone binary.

## Background

Without `vzglyd-get`, installing a slide is:
1. Open browser
2. Navigate to GitHub
3. Find the slide repo
4. Find the latest release
5. Download the .vzglyd file
6. Copy it to /var/lib/vzglyd/slides/
7. Wait for VZGLYD to reload

With `vzglyd-get`, it is:
```
sudo vzglyd get install weather
```

This is a P3 ticket because the manual process works fine and the registry must exist first. Ship the registry and let users manually install for the initial release. Revisit once the registry has traction.

## Commands

```
vzglyd get list                       # List available slides from registry
vzglyd get search <query>             # Search by name or tag
vzglyd get install <name>             # Install latest version
vzglyd get install <name>@<version>   # Install specific version
vzglyd get update                     # Update all installed slides
vzglyd get remove <name>              # Remove an installed slide
vzglyd get info <name>                # Show slide details
```

## Implementation sketch

`vzglyd-get` is a subcommand of the `vzglyd` binary. It is gated behind a `get` subcommand so it doesn't bloat the main binary for users who configure slides manually.

Key operations:
1. Fetch `index.json` from the registry URL (with ETag caching to avoid repeated downloads)
2. Parse the JSON to find matching slides and their release URLs
3. Download the `.vzglyd` file with a progress bar
4. Verify SHA-256 checksum
5. Move the file to the slides directory (default: `/var/lib/vzglyd/slides/`, configurable)
6. Print confirmation

The slides directory path comes from:
1. `--slides-dir` flag
2. `VZGLYD_SLIDES_DIR` environment variable
3. Default: `/var/lib/vzglyd/slides/`

## Registry caching

Cache `index.json` at `/var/cache/vzglyd/registry-cache.json` with a TTL of 1 hour. Include the ETag in the cache file and send `If-None-Match` on subsequent requests — GitHub's CDN supports conditional GET so most runs won't re-download the index.

## Acceptance criteria

- [ ] `vzglyd get list` prints all registry slides with name and description
- [ ] `vzglyd get install clock` downloads and installs clock.vzglyd to slides directory
- [ ] SHA-256 checksum is verified before installation
- [ ] `vzglyd get install` fails with a clear error if the checksum doesn't match
- [ ] `vzglyd get update` updates all installed slides to their latest versions
- [ ] Registry index is cached with ETag to avoid redundant downloads
- [ ] Works without internet connectivity for `list` when cache is warm
- [ ] `--slides-dir` overrides the default install path
