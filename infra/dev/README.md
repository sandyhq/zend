# Build and test container

A machine with everything this repository needs to build and test: CI's
system packages, stable and nightly Rust, cargo-nextest (pinned) and
[kache](https://github.com/kunobi-ninja/kache)
(see [testing.md](../../docs/reference/testing.md); blaze is left out, it fails here). Used on the build server so a
laptop builds nothing.

```sh
infra/dev/run.sh cargo nextest run -p zend-harness          # nightly, kache
NIGHTLY=0 infra/dev/run.sh cargo check -p zend-engine       # stable, as CI
```

- `infra/dev/stage.sh` builds an installable Zend into `target/zend-next`
  ([build-and-deploy.md](../../docs/reference/build-and-deploy.md)).
- Each checkout has its own target volume, `zend-dev-target-<folder>` (`TARGET=` to choose).
- Caches are Docker volumes (`zend-dev-registry`, `zend-dev-git`, `zend-dev-kache` and the target volumes). `docker volume rm` them to start cold.
- Artifacts are in the target volume, not in `target/`. Copy a binary out with
  `infra/dev/run.sh cp /opt/target/release/zend-engine /src/target/`.
- Defaults: nightly with the parallel frontend (`--jobs-frontend=8`), kache on with
  `KACHE_PRESERVE_INCREMENTAL=1` (no full rebuild on the first edit). `NIGHTLY=0`, `KACHE=0`,
  `KACHE_PRESERVE_INCREMENTAL=0` turn them off. Numbers: testing.md.
- Nightly is for the dev loop, not for releases (`stage.sh` builds stable): `rust-toolchain.toml`
  stays on stable.
