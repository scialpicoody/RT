# Ecosystem dependency policy (supersedes ADR 0006)

Maintainer direction is to use the Rust ecosystem. The zero-third-party stance of
[ADR 0006](0006-tiered-dependency-policy.md) is superseded; that ADR remains in place
as history, but its prohibitions no longer govern.

## Adoption ladder

Dependencies are adopted in this order; each rung is tried before falling to the next:

1. **Mature, well-maintained pure-Rust crate.** The default answer.
2. **Vendored fork inside the workspace.** When patching is required or the upstream
   crate carries abandonment risk, fork it into the workspace and own it there.
3. **Own implementation.** Only as a last resort, when no crate or fork is viable.

## Unsafe-code invariant

`#![forbid(unsafe_code)]` holds workspace-wide. Therefore **no FFI-backed dependencies**
(any crate that links or builds C/C++) may be added to the core tiers `rt-core` and
`rt-app`. `rt-gui` may take richer dependencies, including ones with native backends
(e.g. `wgpu`), because the GUI crate is outside the audited core.

## Review gate

Adding a dependency means adding it to the per-tier allowlist below with a one-line
justification, **and** to the allowlist encoded in `.github/workflows/ci.yml`
(`dependency-policy` job, marked `# keep in sync with ADR 0007`). CI enforces exactly
this list; a dependency outside it fails the build with a pointer to this ADR.

First-party workspace crates (e.g. `rt-core`, `rt-app`, `rt-macros`) are always allowed
in any tier and are not listed per-tier below.

## Allowlist

| Tier        | Allowed dependencies                                                                                    |
| ----------- | ------------------------------------------------------------------------------------------------------- |
| `rt-core`   | `rt-macros` (first-party workspace crate)                                                               |
| `rt-app`    | `rt-core`                                                                                               |
| `rt-gui`    | `winit`, `softbuffer`, `taffy`, `vello`, `vello_cpu` (0.3), `wgpu`, `parley` (0.12), `fontique` (0.12) — vello family adopted in #20; `embedded-graphics` retired |
