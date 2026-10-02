# Rust coding standards

The Rust relay (`crates/`, spec #185) follows the **connector's** coding
standards, so nobody switches conventions at the repo boundary:
[`toon-protocol/connector` → `docs/architecture/coding-standards.md`](https://github.com/toon-protocol/connector/blob/main/docs/architecture/coding-standards.md).
That page is the source. This one states which of its rules bind here, and
the rules this repo adds.

## The gate

CI's `rust-gate` job and the factory's gate (`.sandcastle/run-gate.ts`) run
the connector's four commands, in its order, each blocking:

```bash
cargo fmt --all -- --check
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

There is no `rustfmt.toml` and no `clippy.toml`: both tools run on their
defaults, and `-D warnings` makes a clippy warning a failure, tests included.

The gate that decides whether a change is _correct_ is the conformance suite
(`packages/conformance/`), which talks to the built image over the wire. Rust
unit tests are for logic worth testing in isolation.

## Adopted from the connector

- **One error enum per crate**, derived with `thiserror`, its messages written
  for the person reading a log. No catch-all error type: no `anyhow`, no
  `Box<dyn Error>`. Failures a caller must tell apart get separate variants.
- **Refuse to start rather than start wrong.** A setting that is present but
  invalid is a named error at startup, never a silent default.
- **No `.unwrap()` on a production path.** `.expect()` only where the string
  argues why the case is impossible.
- **Fakes, never mocks.** A fake is a working implementation over a simpler
  substrate; a mock asserts on calls and freezes the caller's internals into
  the test.
- **Routers do not own ports.** A function returns an `axum::Router` and the
  binary binds it, so a surface can be driven in a test without a socket.
- **Every module opens with a `//!` block that says why it exists.** Doc
  comments on public items state the contract.
- **Test names are sentences** that state the invariant:
  `toon_secret_key_wins_over_its_alias`, not `test_key_precedence`.
- **Never log a secret key or a mnemonic**, and do not echo one back in an
  error message.

## Added here

- **Unsafe code is forbidden across the workspace.** `Cargo.toml` sets
  `unsafe_code = "forbid"` under `[workspace.lints.rust]` and every crate
  carries `[lints] workspace = true`.
- **The toolchain is pinned, on edition 2024.** `rust-toolchain.toml` names
  one exact release; rustup installs it wherever `cargo` runs (devbox, the
  factory sandbox, CI). The connector builds on floating `stable`; the relay
  does not. `crates/relay/Dockerfile` repeats the version in its `FROM`, so
  bump both together.

- **A rule that can be a type is a type, with a test that the wrong
  construction does not compile.** Each invariant in #185 is a type with one
  constructor: `VerifiedEvent::verify`, `PaymentStatement::stated_on` (visible
  to the paid-write handler alone), `WriteEdge::read`, `SubscribeOffer::read` and
  `TerminatedRoute::confirm`. Its forbidden constructions live in `crates/relay/tests/compile_fail/`, one file each,
  beside the compiler's reason for refusing it, and `trybuild` fails the test
  if one of them builds or fails for a different reason. A `compile_fail`
  doctest would pass on any error, including a renamed import. Regenerate the
  reasons after a deliberate change or a toolchain bump with
  `TRYBUILD=overwrite cargo test -p relay --test compile_fail`.
- **NIP-01 handling is the relay's own.** The relay was built on
  `nostr-sdk`'s `local_relay` behind one adapter module. That framework wraps
  every stream in a WebSocket endpoint whose 128 KiB read buffer it gives no
  way to size, so #237 took the fallback #185 names: `nostr-sdk` and
  `nostr-database` are gone, and `crates/relay/src/session.rs` says what each
  message a client sends is answered with. Only the `nostr` protocol crate remains, on an exact pin and used
  everywhere. The session touches no socket and no store, so its rules are
  tested without either; `crates/relay/src/read_side.rs` does the reading and
  writing.
- **The connector's crate is read for its self-description only.**
  `connector-domain` is a git dependency on one full commit, the one the
  connector image in `deploy/docker-compose.yml` was built from. It also holds
  claim and pricing types, and payment-claim validation lives only in the
  connector, so Rust files name `connector_domain::node` and the settlement
  terms that document lists, each path spelled out, and nothing else.
- **The attribution header names are defined in one module.** Only
  `crates/relay/src/write/payment.rs` spells an `X-TOON-*` header, and only
  the paid-write handler reads a payment statement. Tests state the names
  literally, as the wire does.
- **An invariant type's fields are private to its module.** Not `pub(crate)`:
  the compile-fail tests see the crate from outside, and would not notice.

`deploy/rust-workspace.test.ts` fails the build if the unsafe rule, the
toolchain pin, the no-framework rule, the connector-crate rule, the header rule
or the private-fields rule is undone.

## Not adopted

- Lints stricter than the connector's, and dependency licence or advisory
  checking, are out of scope for the migration (#185).
- The connector's logging rule (`tracing`, structured JSON) is not adopted
  yet. The relay prints the plain lines its operators already read; what its
  log output becomes is decided when a slice gives it something to log.
- The connector's CI check that no `tests/*.rs` harness runs zero tests is not
  copied. The conformance suite is the gate here, and its expected-failure
  markers already fail a case that silently stops testing anything.
