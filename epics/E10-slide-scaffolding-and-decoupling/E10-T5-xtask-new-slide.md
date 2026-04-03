# E10-T5: xtask new-slide Orchestration Command

| Field | Value |
|-------|-------|
| **Epic** | E10 Slide Scaffolding and Project Decoupling |
| **Priority** | P2 (medium) |
| **Estimate** | S |
| **Blocked by** | E10-T1, E10-T3 |
| **Blocks** | - |

## Description

Add a `cargo xtask new-slide <name>` command that wraps `cargo-generate` with project-specific defaults: it generates the slide into `slides/<name>/`, optionally adds it to the workspace members list, and prints next-step instructions. This is the entry point for the "create a new slide" workflow — a single command that handles placement, naming, and workspace integration.

## Background

`cargo-generate` produces a directory wherever you run it. For VZGLYD, new slides must land in `slides/<name>/` and optionally be added to the root `Cargo.toml` workspace members. A raw `cargo generate` invocation requires the author to know the template path, pass `--destination slides/`, and manually edit the workspace file. The xtask wraps this into a project-aware command.

The `cargo xtask` pattern is a Rust convention: a workspace member crate at `xtask/` that exposes project-specific commands via `cargo xtask <subcommand>`. It requires no external tooling beyond what's already in the workspace.

## Step-by-step implementation

### Step 1 — Create the xtask crate

```
xtask/
├── Cargo.toml
└── src/
    └── main.rs
```

`Cargo.toml`:
```toml
[package]
name = "xtask"
version = "0.1.0"
edition = "2024"

[dependencies]
# Minimal: just std process/fs, no heavy deps
```

Add to workspace members in root `Cargo.toml`:
```toml
members = [
    ".",
    "VRX-64-slide",
    "xtask",
    # ... slides ...
]
```

Add the alias in `.cargo/config.toml`:
```toml
[alias]
xtask = "run --package xtask --"
```

### Step 2 — Implement the `new-slide` subcommand

`src/main.rs`:

```rust
fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("new-slide") => new_slide(&args[2..]),
        _ => {
            eprintln!("Usage: cargo xtask new-slide <name> [--with-sidecar]");
            std::process::exit(1);
        }
    }
}
```

The `new_slide` function:

1. Parse `<name>` and optional `--with-sidecar` flag.
2. Verify `cargo-generate` is installed (`which cargo-generate`); if not, print install instructions and exit.
3. Run `cargo generate --path templates/slide --name <name> --destination slides/ --define with_sidecar=<bool>`.
4. Print a summary:
   ```
   Created slides/<name>/

   Next steps:
     1. Add "slides/<name>" to [workspace].members in Cargo.toml (if you want workspace integration)
     2. Edit slides/<name>/src/lib.rs to define your slide geometry and update logic
     3. Edit slides/<name>/manifest.json to declare assets and display config
     4. Run: cd slides/<name> && bash build.sh
     5. Test: cargo run -- --scene slides/<name>
   ```

### Step 3 — Add workspace member insertion (optional)

If the `--workspace` flag is passed, the xtask can programmatically add `"slides/<name>"` to the `[workspace].members` array in the root `Cargo.toml`. This is a convenience — string manipulation on TOML is fragile, so it is acceptable to just print the instruction instead.

If implemented, use a simple line-insertion approach: find the `members = [` line, find the closing `]`, and insert `"slides/<name>",` before it. Do not pull in a full TOML parser for this.

### Step 4 — Test

```bash
cargo xtask new-slide demo_slide
ls slides/demo_slide/
# Verify: Cargo.toml, src/lib.rs, manifest.json, build.sh, shaders/

cargo xtask new-slide api_slide --with-sidecar
ls slides/api_slide/sidecar/
# Verify: sidecar/ directory exists with Cargo.toml, src/main.rs
```

Clean up test slides after verification.

## Acceptance criteria

- [ ] `cargo xtask new-slide foo` generates a slide at `slides/foo/` using the template from E10-T1
- [ ] `cargo xtask new-slide bar --with-sidecar` includes the sidecar scaffold from E10-T3
- [ ] The command prints clear next-step instructions after generation
- [ ] If `cargo-generate` is not installed, the command prints install instructions and exits cleanly
- [ ] The xtask crate compiles with no external dependencies beyond std
- [ ] `cargo xtask` with no arguments prints usage help

## Files to create

| File | Purpose |
|------|---------|
| `xtask/Cargo.toml` | xtask crate manifest |
| `xtask/src/main.rs` | Subcommand dispatch and new-slide implementation |
| `.cargo/config.toml` | Alias `xtask = "run --package xtask --"` |

## Files to modify

| File | Change |
|------|--------|
| `Cargo.toml` | Add `xtask` to workspace members |
