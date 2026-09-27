# Installation

**Homebrew** (macOS/Linux, no Rust toolchain needed — just downloads a
prebuilt binary):

```sh
brew install wattanit/emberly/emberly
```

**Cargo**, if you already have a Rust toolchain (builds from source, pulled
from crates.io):

```sh
cargo install emberly
```

**Prebuilt binary**, no package manager: grab the archive for your platform
from the [latest release](https://github.com/wattanit/emberly-code/releases/latest)
(`x86_64`/`aarch64` musl-static Linux, `x86_64`/`aarch64` macOS), extract it,
and put `emberly` on your `PATH`.

**From source** (needs a recent stable Rust toolchain; **no C toolchain
required**):

```sh
git clone https://github.com/wattanit/emberly-code && cd emberly-code
cargo build --release
# the binary is target/release/emberly — put it on your PATH, e.g.:
install -m 0755 target/release/emberly ~/.local/bin/emberly
```

Once it's installed, head to the [Quick Start](./quickstart.md) to run it for
the first time.
