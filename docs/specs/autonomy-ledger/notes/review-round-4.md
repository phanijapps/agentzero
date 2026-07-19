## Blockers

**1. Forgeable packet trust boundary.** `stores/zbot-conversation/src/domain.rs:212`. Public, deserializable packet fields allowed arbitrary Rust callers to bypass approved-state validation and audit preparation. Fix: make packet construction and fields opaque outside `zbot-conversation`; expose only store-backed approved preparation.

**2. Delimiter injection in system context.** `stores/zbot-conversation/src/domain.rs:295`. Raw packet values could contain a closing ledger tag and escape the declared reference-data block. Fix: render delimiter-safe JSON and test that hostile values cannot create another closing tag.

**3. Unbounded transition audit input.** `gateway/src/http/autonomy.rs:37`. Transition outcomes were persisted without a byte bound. Fix: normalize and bound optional outcome text before persistence and test rejection without an audit write.
