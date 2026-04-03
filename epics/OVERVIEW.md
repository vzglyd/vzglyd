# VZGLYD Engineering Epics

This directory contains the engineering roadmap broken into epics and tickets.
Each epic is a directory containing individual ticket files and an epic summary.

## Completed Epics

| ID | Epic | Status |
|----|------|--------|
| E1 | Renderer Unification | Complete |
| E2 | Scene Transitions | Complete |
| E3 | Shader Portability | Complete |
| E4 | Slide Package Format | Complete |
| E5 | Lifecycle and Data Providers | Complete |
| E7 | Slide Authoring | Complete |
| E8 | Slide Sidecar Model | Complete |
| E9 | Blender Scene Authoring | Complete |

## Active Epics

| ID | Epic | Tickets | Status |
|----|------|---------|--------|
| E10 | [Slide Scaffolding and Project Decoupling](E10-slide-scaffolding-and-decoupling/) | 5 | Not started |
| E11 | [Dashboard Slide Port](E11-dashboard-slide-port/) | 6 | Not started |
| E12 | [OSS Release, Deployment, and Ecosystem](E12-oss-release-and-deployment/) | 15 | Not started |

## E10 Dependency Graph

```
E10-T1 Slide template (cargo-generate)
├──> E10-T3 Sidecar template variant
│    └──> E10-T5 xtask new-slide command
└──> E10-T5 xtask new-slide command

E10-T2 Decouple example slides from engine build
└──> E10-T4 CI build isolation
```

## E12 Dependency Graph

```
E12-T1 ABI stability policy
└──> E12-T2 Publish VRX-64-slide
     ├──> E12-T3 Publish vzglyd_sidecar
     │    └──> E12-T4 Split slides into repos
     │         └──> E12-T5 Prune workspace
     │              └──> E12-T9 Slide distribution CI
     │                   └──> E12-T10 Registry index
     │                        └──> E12-T11 VRX-64-get CLI
     ├──> E12-T13 Slide authoring guide
     └──> E12-T14 rustdoc

E12-T6 Weston kiosk systemd
└──> E12-T7 RPi4 install script (also needs E12-T8)

E12-T8 aarch64 release CI (independent)
E12-T12 README / OSS hygiene (independent)
E12-T15 Browser demo (independent)
```

## Recommended Execution Order

**Phase 1 (parallel tracks):**
- E10-T1 Slide template (independent)
- E10-T2 Decouple example slides (independent)

**Phase 2 (after Phase 1):**
- E10-T3 Sidecar template variant (needs T1)
- E10-T4 CI build isolation (needs T2)

**Phase 3 (after Phase 2):**
- E10-T5 xtask new-slide command (needs T1 and T3)

## Notation

Each ticket file uses the following fields:

- **ID**: `E{epic}-T{ticket}` (e.g., E10-T1)
- **Priority**: P0 (blocker), P1 (high), P2 (medium), P3 (nice-to-have)
- **Estimate**: T-shirt size (S/M/L/XL)
- **Blocked by**: list of ticket IDs that must be completed first
- **Blocks**: list of ticket IDs that depend on this
- **Acceptance criteria**: concrete, testable conditions for done
