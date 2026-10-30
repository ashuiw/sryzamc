# Build requirements

## Rust server

Windows needs the stable Rust MSVC toolchain and the Visual Studio C++ build workload (`link.exe`). If Visual Studio Community is installed without C++, add the compiler/linker with:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --norestart"
```

Open a new Developer PowerShell (or restart the terminal after installation), then run from the repository root:

```powershell
rustup default stable-x86_64-pc-windows-msvc
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
cargo build --release --workspace
```

The Rust server itself does not currently require Java. JNI integration and Java build scripts are not implemented.

## Future JVM bridge

The Pterodactyl launcher is compiled for Java 25 bytecode. Paper lists Java 25 for Minecraft 26.1 and later ([Paper requirements](https://docs.papermc.io/paper/getting-started/)). Building the planned JNI compatibility shims will also require the JDK compiler and JNI headers; running the bridge will require a compatible Java 25+ JVM. The current Rust server binary itself has no Java dependency.
