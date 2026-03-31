# E12: OSS Release, Deployment, and Ecosystem

## Summary

VZGLYD is feature-complete as a private project. This epic prepares it for public release: getting it onto devices without friction, splitting the slide ecosystem into composable repositories, publishing the shared crates to crates.io, and establishing the documentation, hygiene, and discovery infrastructure that gives a healthy open source project its legs.

The target platform for all deployment decisions is a Raspberry Pi 4 running DietPi with Weston in kiosk mode. That is the canonical environment. Everything here is in service of a stranger being able to read the README, follow a single install script, and have a running display within 20 minutes.

## Problem

1. **No installation story.** There is no documented path from a bare RPi4 to a running VZGLYD kiosk. The current working setup (DietPi + Weston + a manually-started binary + slides copied by hand) lives entirely in the developer's head. New users cannot reproduce it.

2. **The repo is a slide kitchen sink.** Every slide lives in the main engine repo as a workspace member. This conflates the engine ABI (stable, slow-moving) with individual slide implementations (fast-moving, externally-contributed). A slide author should be able to create and publish a slide without touching the engine repo at all.

3. **Core crates are unpublished.** `vzglyd-slide` (the ABI contract) and `vzglyd_sidecar` (the networking library) are path-only dependencies. A slide author working in their own repository cannot reference them from crates.io — they must vendor or fork. Publishing these is the prerequisite for the entire third-party slide ecosystem.

4. **No ABI stability commitment.** Nothing documents what `vzglyd-slide` version guarantees mean, which changes are breaking, or what compatibility window the engine maintains. Without this policy, third-party slides break unpredictably on engine updates.

5. **No slide distribution mechanism.** Even after a slide is built, there is no defined way to get it onto a device. No package format publication convention, no registry, no tooling — just `cp slide.vzglyd ~/slides/`.

6. **No OSS hygiene.** No LICENSE file, no CHANGELOG, no contribution guide, no issue templates, no SECURITY policy. These are table stakes for community participation.

## Goals

- A person with a fresh RPi4 + DietPi can run `curl https://... | sh` and have a working VZGLYD kiosk.
- A slide author with no access to the engine repo can create, build, and publish a slide using only `vzglyd-slide` and `vzglyd_sidecar` from crates.io.
- The engine repo contains only the engine and the loading slide. All other slides are in their own repos.
- `vzglyd-slide` and `vzglyd_sidecar` are on crates.io with a documented semver policy.
- Published aarch64 binary artifacts on each engine release.
- A slide registry exists so slides are discoverable.

## Non-goals

- A GUI for slide management.
- Package signing or sandboxed slide installation (future work).
- Supporting platforms other than RPi4/DietPi/Weston for the initial release.
- Automated OTA updates (future work).

## Prerequisites

- E10 (scaffolding and decoupling) complete — needed before splitting repos
- E11 (dashboard slide port) at least at Tier 1 complete — need enough real slides to populate the registry

## Tickets

| ID | Title | Priority | Size | Blocked by | Blocks |
|----|-------|----------|------|------------|--------|
| E12-T1 | ABI stability policy and semver strategy | P0 | S | - | E12-T2 |
| E12-T2 | Publish vzglyd-slide to crates.io | P0 | M | E12-T1 | E12-T3, E12-T4 |
| E12-T3 | Publish vzglyd_sidecar to crates.io | P0 | M | E12-T2 | E12-T4 |
| E12-T4 | Split slides into individual git repositories | P1 | XL | E12-T2, E12-T3 | E12-T5 |
| E12-T5 | Prune vzglyd workspace to engine-only | P1 | S | E12-T4 | - |
| E12-T6 | DietPi + Weston kiosk systemd deployment | P0 | L | - | E12-T7 |
| E12-T7 | RPi4 installation script | P0 | L | E12-T6 | - |
| E12-T8 | aarch64 release CI pipeline | P0 | M | - | E12-T7 |
| E12-T9 | Slide package distribution (CI → .vzglyd → GitHub Releases) | P1 | M | E12-T4 | E12-T10 |
| E12-T10 | Slide registry index | P2 | S | E12-T9 | E12-T11 |
| E12-T11 | vzglyd-get slide installer CLI | P3 | M | E12-T10 | - |
| E12-T12 | README, docs, and OSS hygiene files | P0 | L | - | - |
| E12-T13 | Slide authoring guide | P1 | M | E12-T2 | - |
| E12-T14 | vzglyd-slide and vzglyd_sidecar rustdoc | P1 | M | E12-T2 | - |

## Dependency graph

```
E12-T1 ABI stability policy
└──> E12-T2 Publish vzglyd-slide
     ├──> E12-T3 Publish vzglyd_sidecar
     │    └──> E12-T4 Split slides into repos
     │         └──> E12-T5 Prune vzglyd workspace
     │              └──> E12-T9 Slide distribution CI
     │                   └──> E12-T10 Registry index
     │                        └──> E12-T11 vzglyd-get CLI
     ├──> E12-T13 Slide authoring guide
     └──> E12-T14 rustdoc

E12-T6 Weston kiosk systemd
└──> E12-T7 RPi4 install script
     (also depends on E12-T8 for prebuilt binary URL)

E12-T8 aarch64 release CI (feeds binary URL into T7)

E12-T12 README / OSS hygiene (independent, parallels everything)
```

## Recommended execution order

**Phase 1 (parallel — can start immediately):**
- E12-T1 ABI policy
- E12-T6 Weston/systemd deployment
- E12-T8 aarch64 CI pipeline
- E12-T12 README and OSS hygiene

**Phase 2 (after Phase 1):**
- E12-T2 Publish vzglyd-slide (needs T1)
- E12-T7 Install script (needs T6 + binary URL from T8)

**Phase 3 (after Phase 2):**
- E12-T3 Publish vzglyd_sidecar
- E12-T13 Slide authoring guide
- E12-T14 rustdoc

**Phase 4 (after Phase 3, needs E10+E11 complete):**
- E12-T4 Split slides into repos
- E12-T5 Prune workspace

**Phase 5 (after Phase 4):**
- E12-T9 Slide distribution CI
- E12-T10 Registry index

**Phase 6 (nice-to-have):**
- E12-T11 vzglyd-get CLI
