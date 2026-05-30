# srvcs-isfinite

The finiteness validation primitive of the srvcs.cloud distributed standard
library.

Its single concern: **is the number finite?** JSON numbers cannot encode
infinities or `NaN`, so any value that is a number is finite by construction.
This service is therefore intentionally near-degenerate — it exists for catalog
completeness. It delegates "is this a number" to
[`srvcs-isnumber`](https://github.com/srvcs/isnumber) over HTTP, the single
source of truth for that question, and reports that verdict as its `result`.

If `srvcs-isnumber` is unreachable, `srvcs-isfinite` reports itself **degraded
(503)** rather than guessing.

## API

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/` | Service identity, concern, and dependency list |
| `POST` | `/` | Is `value` finite? |
| `GET` | `/healthz` `/readyz` `/metrics` `/openapi.json` | srvcs service standard surface |

```sh
curl -s -X POST localhost:8080/ -H 'content-type: application/json' -d '{"value": 4}'
# {"value":4,"result":true}
```

Responses:

- `200 {"value": v, "result": bool}` — evaluated; `result` is `srvcs-isnumber`'s verdict.
- `422` — the value is not a number (forwarded from `srvcs-isnumber`).
- `503` — a dependency is unavailable.

## Dependencies

- [`srvcs-isnumber`](https://github.com/srvcs/isnumber) — input validation and the finiteness verdict.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `SRVCS_BIND_ADDR` | `0.0.0.0:8080` | Bind address |
| `SRVCS_ISNUMBER_URL` | `http://127.0.0.1:8081` | Base URL of `srvcs-isnumber` |
| `SRVCS_ENV` | `development` | Environment label for logs |
| `RUST_LOG` | `info,tower_http=info` | Tracing filter |

## Local checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Orchestration tests stand up a mock `srvcs-isnumber` in-process, so the suite
runs without the rest of the fleet. See
[`srvcs/platform`](https://github.com/srvcs/platform) for the shared standard.

> Note: the `cargoHash` in `flake.nix` is inherited from the template and must be
> refreshed with a `nix build` before the Nix gates pass.
