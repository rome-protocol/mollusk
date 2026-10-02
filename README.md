# mollusk

Rome Protocol wrapper around [`mollusk-svm`](https://crates.io/crates/mollusk-svm) for off-chain
SVM execution of arbitrary SBF programs. It is used by the [Rome EVM](https://github.com/rome-protocol/rome-evm)
emulator and by `rome-sdk`, and is kept as a separate crate so those consumers do not need the
on-chain program's internals.

## Layout

- `src/lib.rs` — `Mollusk<'a>` wrapper + `execute_with_sysvar` entry point
- `src/error.rs` — self-contained `MolluskError` (no dependency on `rome_evm::error`)

## Usage

```rust
use mollusk::{Mollusk, error::Result};

let mollusk = Mollusk::new(store, &upgradeable_elf)?;
let result = mollusk.execute_with_sysvar(&ix, None)?;
```

## Build

```
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
cargo fmt --check
```

## License

Copyright © 2024 Coin Vesting Inc. d/b/a Rome Protocol. All rights reserved.
Source-available for personal, non-commercial use; see [LICENSE](LICENSE).
