# Native Windows computer adapter

`colossus-native-computer` implements the typed computer driver port in a
separate Cargo workspace. Its Windows FFI stays outside the safe runtime
workspace, whose unsafe-code prohibition remains unchanged. Other platforms
return capability unavailable.

Trusted CLI and sidecar composition supply the verified current executable and
protected host/controller process identities. The fixed private helper mode
starts before ordinary command parsing and authenticates its private channel.
Run bindings, selected native window identities, fresh observations and a single
writer lease constrain capture, accessibility and input. Helper ownership and
input cleanup remain native lifecycle responsibilities.

Portable validation runs through `cargo xtask check rust` and
`cargo xtask check desktop`. Dependency policy scans this workspace's own
`Cargo.lock` as well as the root application graphs.

The explicit Windows acceptance harness uses only synthetic sibling windows and
requires an unlocked interactive desktop with a non-elevated user token.
See [computer control acceptance](../../docs/develop/testing.md#windows-computer-control-acceptance)
for exact commands, fixture ownership, evidence and CI prerequisites.
