---
---

Remove `packages/core`, `docker/`, and `packages/client` `COPY`/build steps from `packages/bls/Dockerfile` and `Dockerfile.bootstrap`, since those workspace packages no longer exist post-polyrepo-split.
