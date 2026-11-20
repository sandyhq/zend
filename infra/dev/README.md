# Build and test container

A machine with everything this repository needs to build and test: CI's
system packages, stable and nightly Rust, cargo-nextest (pinned) and
[kache](https://github.com/kunobi-ninja/kache)
(see [testing.md](../../docs/reference/testing.md); blaze is left out, it fails here). Used on the build server so a
laptop builds nothing.

```sh
infra/dev/run.sh cargo nextest run -p zend-harness          # stable, with kache
NIGHTLY=1 infra/dev/run.sh cargo check -p zend-engine       # nightly, parallel frontend
```

- `infra/dev/stage.sh` builds an installable Zend into `target/zend-next`
  ([build-and-deploy.md](../../docs/reference/build-and-deploy.md)).
- Each checkout has its own target volume, `zend-dev-target-<folder>` (`TARGET=` to choose).
- Caches are Docker volumes (`zend-dev-registry`, `zend-dev-git`, `zend-dev-kache` and the target volumes). `docker volume rm` them to start cold.
- Artifacts are in the target volume, not in `target/`. Copy a binary out with
  `infra/dev/run.sh cp /opt/target/release/zend-engine /src/target/`.
- Nightly is for measuring and the dev loop, not for releases: `rust-toolchain.toml`
  stays on stable.
