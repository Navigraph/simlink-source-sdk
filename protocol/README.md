# Protocol manifest

`source-schema.json` is Simlink's source manifest. Simlink generates it, and a workflow in the Simlink repository opens
a pull request here whenever it changes. Don't edit it by hand: the next update would overwrite the change.

A pull request that adds a channel also needs that channel's types listed in
`crates/simlink-source-sdk/tests/schema.rs`, which fails until they are.
