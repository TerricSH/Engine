# Minimal Game

This project is the engine's executable project-workflow sample.

Its startup scene uses a cooked mesh plus an opaque cooked material backed by
the cooked checker texture; it does not rely on the built-in default material.

```powershell
cargo run -p sandbox -- project check examples/minimal-game
cargo run -p sandbox -- project cook examples/minimal-game
cargo run -p sandbox -- game examples/minimal-game --headless --frames 3
cargo run -p sandbox --features backend-vulkan,tooling-editor -- editor examples/minimal-game
```
