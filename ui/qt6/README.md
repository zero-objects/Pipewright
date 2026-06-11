# Pipewright — Qt6 frontend

Qt6 (C++/QML) desktop UI for the Pipewright pipeline toolkit. The
Rust backend is exposed through `crates/pipeline-ffi` as a stable C
ABI; CMake links the resulting `cdylib` via Corrosion.

![First window](docs/screenshot.png)

## Prerequisites

| Platform | One-shot install |
|---|---|
| macOS | `brew install qt cmake` |
| Debian/Ubuntu | `apt install qt6-base-dev qt6-declarative-dev qt6-svg-dev cmake` |
| Fedora | `dnf install qt6-qtbase-devel qt6-qtdeclarative-devel qt6-qtsvg-devel cmake` |
| Windows | Qt online installer (6.5+) + Visual Studio 2022 + CMake |

You also need a recent stable Rust toolchain (matching the workspace
`rust-toolchain.toml`). Corrosion is fetched automatically.

## Build

```bash
cmake -S ui/qt6 -B ui/qt6/build
cmake --build ui/qt6/build -j
```

The output binary lives at:

- macOS: `ui/qt6/build/pipewright-ui.app`
- Linux: `ui/qt6/build/pipewright-ui`
- Windows: `ui/qt6/build/Debug/pipewright-ui.exe`

## Run

```bash
# macOS
open ui/qt6/build/pipewright-ui.app
# Linux
./ui/qt6/build/pipewright-ui
```

The first window has a paste area for a pipeline definition and a
"Detect platform" button — a smoke test that the FFI bridge is alive
and that all three platforms (GitLab, GitHub, Jenkins) are detected
correctly.

## Layout

```
ui/qt6/
├── CMakeLists.txt        ← Qt6 + Corrosion glue
├── README.md             ← this file
└── src/
    ├── main.cpp          ← entry point, loads Main.qml
    ├── BridgeApi.{h,cpp} ← Q_INVOKABLE wrappers over pipeline_ffi.h
    └── qml/
        └── Main.qml      ← initial shell
```

`ui/qt6/` is **outside the Cargo workspace** on purpose: Cargo doesn't
need to know about Qt, and CMake doesn't need to know about Rust.
Corrosion is the one bridge that does.

## Troubleshooting

- **`Qt6 not found`**: Set `CMAKE_PREFIX_PATH` to your Qt install
  (e.g. `-DCMAKE_PREFIX_PATH=/opt/Qt/6.7.0/macos`). On macOS, Homebrew
  Qt is auto-detected.
- **Corrosion fetch fails behind a proxy**: clone Corrosion locally
  and set `-DFETCHCONTENT_SOURCE_DIR_CORROSION=/path/to/corrosion`.
- **`pipeline_ffi.h not found`**: the header lives in
  `crates/pipeline-ffi/include/`. Run `cargo build -p pipeline-ffi`
  once if you skipped a CMake step.
