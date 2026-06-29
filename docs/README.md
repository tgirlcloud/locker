## Locker

Locker is a tool designed to lint your flake.lock file to find duplicate entries by their flake uri.

![demo image](./image.png "demo image")

### Usage

```bash
locker <flake-lock-file>
```

### treefmt

`locker` can run as a [treefmt](https://github.com/numtide/treefmt) formatter.
It checks rather than rewrites files, so a non-zero exit reports the check as
failed.

```toml
[formatter.locker]
command = "locker"
includes = ["flake.lock"]
```

treefmt passes every matched `flake.lock` as a positional argument; `locker`
checks each one and fails the run if any contains duplicate inputs.

### GitHub Action

```yaml
name: Validate Flake Lock

on:
  workflow_dispatch:
  push:
    paths:
      - "**.lock"

jobs:
  check-flake:
    name: Check Lock
    runs-on: ubuntu-latest

    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Install Nix
        uses: cachix/install-nix-action@v31

      - name: Check flake.lock
        run: nix run github:tgirlcloud/locker
```
