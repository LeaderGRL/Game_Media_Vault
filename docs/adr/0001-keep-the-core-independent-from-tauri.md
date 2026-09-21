# Keep the core independent from Tauri

Game Media Vault will implement domain rules, acquisition orchestration, persistence, connectors, and media processing in Rust crates that do not depend on Tauri. Tauri and the CLI are separate frontends over the same application layer because acquisitions may run for hours or days, including unattended operation on a server or NAS, and coupling the engine to the desktop lifecycle would make those workflows unnecessarily fragile and expensive to separate later.
