# Architecture decision records

Decisions about the relay's **Nostr** behaviour live here: what it serves on its read
port, how readers interpret events, and the UIs it hosts. Decisions about ILP, payment,
claims and settlement stay in the connector's
[`docs/adr/`](https://github.com/toon-protocol/connector/tree/main/docs/adr); a ticket
that cites one by number means that one.

| ADR | Decision |
| --- | --- |
| [0001](0001-app-uis-live-on-the-relay.md) | App UIs live on the relay |
| [0002](0002-readers-honour-repo-state-only-from-authorized-authors.md) | Readers honour repo state only from authorized authors |
