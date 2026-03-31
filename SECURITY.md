# Security Policy

## Supported versions

| Version | Supported |
| --- | --- |
| latest | yes |

## Reporting a vulnerability

Do not open a public GitHub issue for security-sensitive reports.

Use GitHub Security Advisories:

<https://github.com/vzglyd/vzglyd/security/advisories/new>

If private advisories are unavailable, contact the maintainers directly before publishing details.

## Threat model

VZGLYD loads `.vzglyd` packages from the local filesystem and executes slide code inside a WebAssembly sandbox via wasmtime. Slides do not get direct filesystem or arbitrary network access unless the engine explicitly exposes it.

Optional sidecars run as separate WASI modules and are responsible for external data fetching. They are still untrusted inputs and should be treated accordingly.

The sandbox does not protect against every class of bug. In particular:

- malicious or malformed slide assets may still exercise parser bugs
- shader code may trigger GPU-driver defects
- downloaded packages should be treated as untrusted until verified

Load only slides you trust, and prefer release artifacts with published checksums.
