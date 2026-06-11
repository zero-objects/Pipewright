# Install

Pipewright ships as:

- a single CLI binary (`pipewright`) — detect / inspect / render /
  capabilities / migrate / recipe compose+apply / plan, and `run`
  when Docker is available
- a Qt6 desktop application (`pipewright-ui`) — every CLI feature in
  a GUI plus interactive DAG navigation
- a C-ABI shared library (`libpipeline_ffi`) — for integrating the
  toolkit into other host languages

This page covers building each from source. Pre-built installers
(`.dmg` / `.deb` / `.rpm` / `.msi`) are produced by the release
pipeline on tagged releases; once a tag is cut they appear on the
project's Releases page.

## Prerequisites

| For | You need |
|---|---|
| CLI only | Rust 1.88+ (any platform `cargo` supports) |
| Local `run` subcommand | Docker daemon reachable (defaults to `unix:///var/run/docker.sock`) |
| Qt6 desktop UI | Rust 1.88+, CMake 3.24+, Qt 6.5+ (Core / Gui / Quick / Qml / QuickControls2 / Svg), a C++20 compiler |

On macOS the Qt build also pulls Corrosion via `FetchContent`, so you
need network access on first configure (cached afterwards).

### Installing the prerequisites

| OS | One-shot install |
|---|---|
| macOS | `brew install rustup qt cmake` (then `rustup-init`) |
| Debian/Ubuntu | `sudo apt install build-essential cmake qt6-base-dev qt6-declarative-dev qt6-svg-dev curl` then `curl https://sh.rustup.rs -sSf \| sh` |
| Fedora | `sudo dnf install gcc-c++ cmake qt6-qtbase-devel qt6-qtdeclarative-devel qt6-qtsvg-devel` then rustup |
| Windows | Install Visual Studio 2022 (C++ workload), Qt 6.7+ via the Qt Online Installer, CMake, and rustup |

## Build the CLI

From the repo root:

```bash
cargo build --release -p pipeline-cli
```

The binary lands at `target/release/pipeline`. Put it on your PATH or
invoke it by path. To check it works:

```bash
target/release/pipewright --help
```

## Build the desktop UI

```bash
cmake -S ui/qt6 -B ui/qt6/build
cmake --build ui/qt6/build -j
```

Output:

- macOS: `ui/qt6/build/pipewright-ui.app` (double-clickable bundle)
- Linux: `ui/qt6/build/pipewright-ui` (binary)
- Windows: `ui/qt6/build/Release/pipewright-ui.exe`

The CMake project pulls in `pipeline-ffi` automatically via Corrosion,
so you don't need to run `cargo build` separately for the UI.

## Build the FFI library

```bash
cargo build --release -p pipeline-ffi
```

Outputs:

- `target/release/libpipeline_ffi.{so,dylib,dll}` — shared library
  consumers link against
- `crates/pipeline-ffi/include/pipeline_ffi.h` — auto-generated C
  header (also committed for consumers who don't run cargo)

See [`crates/pipeline-ffi/src/lib.rs`](../../crates/pipeline-ffi/src/lib.rs)
for the exported functions; every call returns a JSON string or a
shape `{"error": "..."}` on failure.

## Installer packages (release builds)

The `ui/qt6/CMakeLists.txt` is wired for CPack:

```bash
cmake -S ui/qt6 -B ui/qt6/build -DCMAKE_BUILD_TYPE=Release
cmake --build ui/qt6/build --config Release -j
cd ui/qt6/build
cpack -C Release
```

CPack will produce, depending on platform:

- macOS — `pipewright-<version>-Darwin-arm64.dmg` (DragNDrop)
- Linux — `.deb` and `.rpm`
- Windows — `.msi` (WIX Toolset required)

Signed/notarised builds are produced by the GitHub Actions release
workflow (`.github/workflows/release.yml`) when secrets are present.

## Verify the install

A working install should pass this smoke:

```bash
echo 'build:
  script:
    - cargo build' > /tmp/demo.yml

pipewright inspect /tmp/demo.yml
pipewright render /tmp/demo.yml --format svg > /tmp/demo.svg
```

You should see one-job pipeline JSON and an SVG you can open in any
viewer. The repo also carries a headless end-to-end smoke for the UI:
`ui/qt6/scripts/smoke.sh` (builds required first).

## Uninstall

The build is fully out-of-tree. Remove `target/`, `ui/qt6/build/`,
and any installed packages with your OS's package manager.

## Next

- **[Quickstart](quickstart.md)** — work through a real pipeline
- **[User manual](manual.md)** — every subcommand and UI tab in detail
