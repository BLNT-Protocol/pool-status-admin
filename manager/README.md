# Pool Status Manager

Admin for Blend pools with two roles: an **owner** with every pool admin
power, and an **operator** that can only set pool status.

The status-only contract in the repository root is unchanged; this crate is
a separate contract with its own releases (`manager-v*` tags).

## Why

The root contract makes status the only admin power left, for good: reserve
caps, rates and the admin itself can never change again.  Some pools need
status changed automatically (e.g. a cron job opening and freezing the pool
on a schedule) and still need occasional config changes.  Here the hot key
that runs the schedule is the operator, and everything else stays with the
owner, meant to be a multisig.

## Roles

| Function | Owner | Operator |
|---|---|---|
| `set_status(caller, pool, pool_status)` | yes | yes |
| `accept_admin(pool)` | yes | |
| `propose_admin(pool, new_admin)` | yes | |
| `update_pool(pool, backstop_take_rate, max_positions, min_collateral)` | yes | |
| `queue_set_reserve(pool, asset, metadata)` | yes | |
| `cancel_set_reserve(pool, asset)` | yes | |
| `set_emissions_config(pool, res_emission_metadata)` | yes | |
| `set_operator(operator)` | yes | |

`set_status` takes the caller explicitly; it must be the owner or the
operator and must sign.  Each other call passes straight through to the
pool, whose own rules (status thresholds, the reserve timelock) still apply.
`set_reserve` is permissionless on the pool and needs no admin.

A stolen operator key can only change the status of the pools this contract
administers; the owner replaces the operator with `set_operator`.

There is no ownership transfer and no renounce.  A multisig owner rotates its
own signers; to move a pool to a different admin, the owner calls
`propose_admin` and the new admin accepts.  The contract cannot be upgraded.
One deployment can administer any number of pools.

The pool interface is the same in Blend v2 and BLNT v3.

## Usage

```
stellar contract deploy --wasm-hash <hash> -- --owner <multisig> --operator <hot key>
stellar contract invoke --id <pool> -- propose_admin --new_admin <this contract>                      # pool admin
stellar contract invoke --id <this contract> -- accept_admin --pool <pool>                             # owner
stellar contract invoke --id <this contract> -- set_status --caller <operator> --pool <pool> --pool_status 4   # operator
```

## Build and test

```
make build      # stellar contract build --locked
make test
```

Rust is pinned to 1.91.1 (`rust-toolchain.toml`).

## License

MIT, see [LICENSE](../LICENSE).
