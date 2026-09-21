# Windows application switching validation

Validated on 2026-09-20 on the authorized Windows 11 x64 host with the pinned
`1.14.1-socks-proxy.2` core.

The production application controller serializes switches through exclusive
mutable ownership. Each transaction validates the candidate, requires explicit
confirmation before a core restart, applies the runtime configuration, and only
then persists the new revision. Runtime or persistence failures invoke the old
configuration rollback. A rollback failure clears the claimed applied mode and
enters `Error`.

`scripts/validation/windows-application-switch-smoke.ps1` ran a Rust validation
executable against two real local SOCKS5 sing-box upstreams. Upstream A and B
overrode the same requested destination to independent HTTP fixtures returning
`A` and `B`, so the response body proved the path used by each new connection.

Observed result:

```json
{"path_a":true,"path_b":true,"rollback_b":true,"restart_warning":true,"direct_stopped":true}
```

The sequence was:

1. A restart plan without confirmation was rejected before changing runtime.
2. A confirmed Rules switch selected proxy A; a new connection returned `A`.
3. A confirmed Global Proxy switch selected proxy B; a new connection returned
   `B`.
4. The next restart was deliberately failed after stopping the client core. The
   controller restored B and retained Global Proxy as the applied mode; a new
   connection again returned `B`.
5. A confirmed Direct switch stopped the client core, and its listening port was
   closed.

All restricted runtime configuration files were removed after the processes
stopped. Unit tests separately cover candidate validation failure, runtime
failure, persistence failure, successful rollback, and rollback failure.
