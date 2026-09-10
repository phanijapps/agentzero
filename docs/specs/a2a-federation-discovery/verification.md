# A2A Federation Verification

## Automated evidence

The implementation is exercised without provider spend by temporary SQLite
stores, the production Axum router, a real loopback Reqwest client, and a fake
terminal A2A peer:

```bash
cargo test -p gateway-a2a
cargo test -p discovery
cargo test -p gateway --test a2a_http
cargo test -p gateway --test a2a_outbound
cargo test -p gateway trust_authorization_tests --lib
cargo test -p gateway-execution a2a_tools_are_root_and_ward_only --lib
cargo test -p agent-runtime peer_safe_schema_exposes_only_respond --lib
cargo test -p agent-runtime persisted_remote_result_taints_only_system_marked_history --lib
```

The real-HTTP router test proves A2A version/media/auth headers and send/get
interoperability over a random loopback port. The outbound worker test proves
persist-before-return, stable deduplication, dispatch-to-poll correlation,
live steering acknowledgement, and the durable continuation fallback when the
originating turn has already completed. The delivery-authorization test proves
that removing trust or changing a peer's target prevents queued inbound work
from starting.

## Two-daemon operator journey

Use isolated data directories and ports:

```bash
zbotd --data-dir /tmp/zbot-a --http-port 18791 --a2a \
  --a2a-public-base-url http://127.0.0.1:18791
zbotd --data-dir /tmp/zbot-b --http-port 18792 --a2a \
  --a2a-public-base-url http://127.0.0.1:18792
```

In separate shells, create reciprocal credentials. Each issued token is shown
once and is installed only in the opposite zBot's outbound trust record:

```bash
zbot --data-dir /tmp/zbot-b peers issue peer-a --target-agent root
zbot --data-dir /tmp/zbot-a peers add peer-b \
  --origin http://127.0.0.1:18792 --token '<token-issued-by-b>' \
  --target-agent root

zbot --data-dir /tmp/zbot-a peers issue peer-b --target-agent root
zbot --data-dir /tmp/zbot-b peers add peer-a \
  --origin http://127.0.0.1:18791 --token '<token-issued-by-a>' \
  --target-agent root
```

Discovery can be inspected without granting trust:

```bash
zbot peers discover --wait-seconds 3
```

Then ask zBot A:

```text
List the trusted zBots. Delegate this bounded task to peer-b: compare three
practical ways to reduce household standby power, then continue helping me
locally while it runs. When the remote result arrives, summarize it and do not
take any external action.
```

Expected behavior: delegation immediately returns a stable queued task ID;
the local turn remains usable; the remote result is visibly attributed and
treated as untrusted; if the local turn ends first, a respond-only continuation
delivers it. Get/list/cancel with the other peer's credential must return the
same not-found response as an unknown task.

## External conformance status

The supported surface is mechanically checked against
`contracts/openapi/a2a-federation.yaml` and the official `a2a-lf` 1.0 wire
types. An external A2A CLI/TCK was not installed or run in this change; that is
a release-validation step tracked by
[`a2a-external-conformance`](../../backlog.md#a2a-external-conformance), not
claimed evidence here. Streaming, push notifications, extended cards, files,
structured parts, and subscriptions are intentionally unsupported and
advertised as such.
