# E12-T12: README, Documentation, and OSS Hygiene Files

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P0 (blocker) |
| **Estimate** | L |
| **Blocked by** | - |
| **Blocks** | - |

## Description

Write or update all the files that make a GitHub project look maintained and trustworthy: the top-level README, CONTRIBUTING.md, CHANGELOG.md, SECURITY.md, CODE_OF_CONDUCT.md, LICENSE, and GitHub issue/PR templates. This is not about vovzglyd of words — it is about a new visitor's first 60 seconds being productive rather than confusing.

## Background

The README is the product's front page. An open source project with no README, no license, or no contribution guide signals abandonment before anyone reads a line of code. These files are table stakes and should exist before any public announcement.

## Files to create or overwrite

### LICENSE

Choose and commit a license before doing anything else. The Rust ecosystem standard is dual MIT/Apache-2.0:

```
Licensed under either of

 * Apache License, Version 2.0 (LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license (LICENSE-MIT or http://opensource.org/licenses/MIT)

at your option.
```

Create `LICENSE-MIT` and `LICENSE-APACHE` at the repo root. All slide repos, `vzglyd-slide`, and `vzglyd_sidecar` use the same license.

**This must match the license declared in every Cargo.toml.** If Cargo.toml says `MIT OR Apache-2.0`, the actual license files must exist.

### README.md (top-level)

Structure:

```markdown
# VZGLYD

[screenshot or demo GIF here — a real photo of the display on a TV is more compelling than a screenshot]

A Raspberry Pi display engine for always-on ambient information dashboards. Build slides in Rust, compile to WebAssembly, run on your TV.

## What is VZGLYD?

[2–3 sentences. What it does, what hardware it runs on, what it produces.]

## Quick start

[The curl | sh install command. Nothing else. Make this the first thing someone can do.]

## Slides

[Table of official slides with one-line descriptions and whether they need config.]

## Building your own slide

[One paragraph + link to the full authoring guide in docs/]

## Architecture

[One diagram showing: slides (WASM) ← engine → display (Wayland/KMS)
  and: slide source → cargo build → .vzglyd package → engine loads at runtime]

## Development

[Prerequisites, how to run a slide locally, how to run tests.]

## Contributing

[Link to CONTRIBUTING.md]

## License

[Dual MIT/Apache-2.0]
```

**Do not write walls of text.** The README should be skimmable in 30 seconds. The full detail belongs in `docs/`.

### CONTRIBUTING.md

```markdown
# Contributing to VZGLYD

## Ways to contribute

- **Build a slide** — The best contribution is a new slide. See the [slide authoring guide](docs/authoring-guide.md).
- **Report bugs** — Use [GitHub Issues](...). Include your DietPi version, VZGLYD version, and the slide that failed.
- **Fix bugs** — PRs welcome. See the development workflow below.
- **Improve documentation** — Especially the authoring guide and deploy docs.

## Development workflow

[How to set up the dev environment, run tests, run clippy]

## Pull request guidelines

- One logical change per PR
- All tests must pass (`cargo test`)
- `cargo clippy -- -D warnings` must pass
- No new `unsafe` without explanation
- ABI-breaking changes to `vzglyd-slide` require a MAJOR version bump and a CHANGELOG entry

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
```

### CHANGELOG.md

Start with a minimal entry:

```markdown
# Changelog

All notable changes to VZGLYD are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/).
Versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] — 2026-XX-XX

### Added
- Initial open source release
- WASM slide loading via wasmtime
- DRM/KMS and Wayland display backends
- wgpu-based rendering with WGSL shaders
- Slide sidecar model for live data
- vzglyd-slide 0.1.0 ABI
- vzglyd_sidecar 0.1.0
```

### SECURITY.md

```markdown
# Security Policy

## Supported versions

| Version | Supported |
|---------|-----------|
| latest  | yes       |

## Reporting a vulnerability

Please do not open a public GitHub issue for security vulnerabilities.

Email: [maintainer email or GitHub Security Advisory link]

I will respond within 7 days. If the vulnerability is confirmed, I will work on a fix and coordinate disclosure.

## Threat model

VZGLYD loads `.vzglyd` packages (WebAssembly + assets) from the local filesystem. The WASM sandbox (via wasmtime) constrains what a slide can do: slides cannot access the filesystem, network, or system calls beyond what the engine explicitly permits. Sidecars run with WASI capabilities limited to network sockets.

Packages loaded from untrusted sources (e.g., internet downloads) are executed in the sandbox. However, the sandbox does not protect against malicious shader code or assets that exploit GPU driver bugs — only load slides you trust.
```

### CODE_OF_CONDUCT.md

Adopt the Contributor Covenant 2.1 verbatim. This is the de facto standard and adds no maintenance burden.

### GitHub issue templates (.github/ISSUE_TEMPLATE/)

**bug_report.yml:**

```yaml
name: Bug report
description: Something isn't working
labels: [bug]
body:
  - type: input
    id: vzglyd_version
    attributes:
      label: VZGLYD version
      placeholder: "0.1.0"
    validations:
      required: true
  - type: input
    id: slide
    attributes:
      label: Affected slide (if applicable)
      placeholder: "weather, or 'all'"
  - type: textarea
    id: description
    attributes:
      label: What happened?
    validations:
      required: true
  - type: textarea
    id: logs
    attributes:
      label: Relevant log output
      description: "Run with RUST_LOG=debug and paste the output"
      render: text
```

**slide_request.yml:**

```yaml
name: Slide idea
description: Request a new official slide
labels: [enhancement, slide-request]
body:
  - type: textarea
    id: data_source
    attributes:
      label: What data would this slide show?
    validations:
      required: true
  - type: textarea
    id: api
    attributes:
      label: What API or data source does it use?
  - type: dropdown
    id: willing_to_build
    attributes:
      label: Are you willing to build this slide?
      options: ["Yes", "No", "Maybe with guidance"]
```

### Pull request template (.github/pull_request_template.md)

```markdown
## What does this PR do?

## How was this tested?

## Checklist

- [ ] `cargo test` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] If this changes `vzglyd-slide`: CHANGELOG entry and version bump
- [ ] If this is a new slide: follows the authoring guide structure
```

### docs/ directory

Create a `docs/` directory with:
- `docs/architecture.md` — engine internals (slide loader, renderer, sidecar lifecycle)
- `docs/shader-contract.md` — move/update `SHADER_CONTRACT.md` here
- Link placeholders for the authoring guide (E12-T13) and rustdoc (E12-T14)

## Acceptance criteria

- [ ] `LICENSE-MIT` and `LICENSE-APACHE` exist at repo root
- [ ] `README.md` has: screenshot/GIF, quick install command, slides table, architecture diagram, links to docs
- [ ] `CONTRIBUTING.md` explains how to build a slide, run tests, and open a PR
- [ ] `CHANGELOG.md` has an initial 0.1.0 entry
- [ ] `SECURITY.md` describes the threat model and contact for vulnerabilities
- [ ] `CODE_OF_CONDUCT.md` adopts Contributor Covenant 2.1
- [ ] `.github/ISSUE_TEMPLATE/bug_report.yml` exists
- [ ] `.github/ISSUE_TEMPLATE/slide_request.yml` exists
- [ ] `.github/pull_request_template.md` exists
- [ ] `docs/architecture.md` gives a high-level engine overview
- [ ] All Cargo.toml `license` fields match the actual license files

## Files to create

| File | Purpose |
|------|---------|
| `LICENSE-MIT` | MIT license text |
| `LICENSE-APACHE` | Apache 2.0 license text |
| `README.md` | Project README (overwrite existing if any) |
| `CONTRIBUTING.md` | Contribution guide |
| `CHANGELOG.md` | Release history |
| `SECURITY.md` | Vulnerability reporting policy |
| `CODE_OF_CONDUCT.md` | Contributor Covenant 2.1 |
| `.github/ISSUE_TEMPLATE/bug_report.yml` | Bug report template |
| `.github/ISSUE_TEMPLATE/slide_request.yml` | Slide request template |
| `.github/pull_request_template.md` | PR template |
| `docs/architecture.md` | Engine architecture overview |
