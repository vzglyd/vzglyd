# Bridges

The Constraint Principle is honest about what it cannot do. Some data sources — iCloud CloudKit, complex OAuth flows, proprietary authentication protocols — cannot be reached from within a WASM sidecar's network constraints. The bridge is not a failure of the system; it is the system's honest acknowledgment of its own limits.

External bridge adapters are the acknowledged places where the constraint cannot hold and accommodation is made. The pattern is deliberately simple. An external process handles the complex authentication or proprietary protocol, writes a compact JSON document to a well-known host path, and a tiny VZGLYD sidecar reads that file through a read-only WASI preopen and forwards the bytes over the slide channel.

The bridge writes to a path. The sidecar reads from that path. The data crosses the boundary.

## The Reminders Bridge

The reminders bridge is the reference implementation. It demonstrates the full bridge pattern: a producer that owns everything the system cannot own, and a sidecar that owns only what the system can.

The Python process owns Apple authentication, 2FA, and CloudKit quirks. The VZGLYD sidecar owns only efficient file polling, JSON validation, and channel delivery. This keeps the runtime boundary clear and makes the slide itself independent of the upstream fetch mechanism. The slide does not know how the data arrived. It knows only that data arrives.

The host directory containing the produced JSON is declared in the slide manifest under `sidecar.wasi_preopens`. That declaration is the mapping: a host path becomes a guest-visible path. The sidecar reads from the guest-visible path. The producer writes to the host path. The mapping is the bridge's contract.

## Adding Another Bridge

To add another bridge, write a producer that emits stable JSON, choose a host directory to expose, declare that host-to-guest mapping in the slide manifest `sidecar.wasi_preopens`, and keep the sidecar logic read-only. The slide author who works within these conditions is not working in a reduced medium. They are working in a specific medium with specific formal properties — and the bridge is one of those properties: a named, deliberate exception to the constraint, not a violation of it.

If a future Rust or WASM-native client becomes practical, the slide can keep its payload format and swap only the sidecar.
