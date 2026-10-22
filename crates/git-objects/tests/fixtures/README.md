Real objects from a small throwaway git repo (two roots of history joined by
a merge, one annotated tag). For each object id `<id>`:

- `<id>.loose`: the file git wrote under `.git/objects/`, zlib-compressed
- `<id>.type`: `git cat-file -t`
- `<id>.body`: `git cat-file <type> <id>`, the body without the header

`MERGE`, `TAG` and the rest are named in `tests/objects.rs`.
