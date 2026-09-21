# Windows core configuration validation

Validated on 2026-09-20 against the authorized Windows 11 x64 host at
`10.168.1.158`. Test credentials were fixture-only values and were not written
to repository files or command arguments.

## Locked artifacts

- sing-box version: `1.14.1-socks-proxy.2`
- sing-box SHA-256: `40a64f2973858203468da544db40c6d546f14fd8f02ffa38954e16bb776927a1`
- Rust target: `x86_64-pc-windows-msvc`
- validation executable SHA-256: `2a29c3b4eaddd7db846ca033b74402b31abf4a8a7afcf87e393efe418d6f3bbc`

## Procedure

1. Cross-build `examples/windows_core_config_smoke.rs` with `cargo-xwin`.
2. Copy the validation executable and the locked patched core to the Windows
   validation workspace.
3. Run `scripts/validation/windows-config-compile-smoke.ps1` over the existing
   SSH transport.
4. The Rust executable compiles a rules-mode configuration containing a SOCKS5
   credential, mixed ports, a domain-suffix rule, direct DNS, base exceptions,
   and strict FakeIP cache settings.
5. It creates the config with a protected DACL, runs `sing-box check -c`, checks
   the exact ACL principals, inspects the child arguments, and drops the RAII
   temporary file.

Observed result:

```json
{"config_check":true,"acl_current_user_and_system":true,"secret_in_args":false,"temporary_removed":true}
```

The validator captures core stdout and stderr instead of forwarding raw core
output to application logs. `ProxyCredentials` and `CompiledCoreConfig` redact
their `Debug` output, and their owned buffers are overwritten on drop. The
generated config is the only child-process input containing the fixture
password; the child command contains only `check`, `-c`, and the restricted
temporary path.

## Additional checks

- All 59 Rust tests passed.
- `cargo fmt --check` passed.
- `cargo clippy --locked --all-targets -- -D warnings` passed.
- Windows MSVC `cargo check --locked --all-targets` passed.
- The original unpatched `v1.14.1` binary rejected `strict_mode`, while the
  pinned project build accepted it. This confirms the validation used the
  required patched version.
