# CADEgg

CADEgg is a Windows desktop AutoCAD assistant built with Tauri, React, and Rust. The app provides a compact floating chat panel, sends structured tool calls to the Rust backend, and drives AutoCAD through an internal .NET bridge with COM fallback.

## Current Status

This is still an early prototype. The practical provider path is GLM or Gemini. Claude appears in the settings UI for future support, but Claude tool-use is not implemented yet.

## Requirements

- Windows
- AutoCAD with COM automation available
- Node.js and npm
- Rust toolchain with Cargo
- API key for GLM or Gemini

## Development

```powershell
npm install
npm run build
npm run tauri dev
```

If PowerShell blocks `npm.ps1`, run `npm.cmd` instead or adjust the local execution policy.

## Verification

```powershell
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

Some Rust tests require a local AutoCAD installation and are marked ignored.

## Repository Hygiene

Generated directories such as `node_modules`, `dist`, `src-tauri/target`, and `src-tauri/gen/schemas` should not be committed.