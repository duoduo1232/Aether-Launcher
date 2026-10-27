# Changelog

All notable changes to Aether Launcher will be documented in this file.

The project follows [Semantic Versioning](https://semver.org/) once public releases begin.

## [Unreleased]

### Added

- Rust workspace with separated launcher domain crates
- Tauri 2 desktop shell
- React, TypeScript, and Fluent UI v9 interface foundation
- Frontend lint, tests, and production build
- Rust formatting, linting, and unit-test checks
- Issue Forms, Discussion Forms, pull request template, and CODEOWNERS
- Dependabot, CodeQL, dependency review, labeling, CI, and draft release workflows
- Sidebar navigation between home, instances, library, accounts, and settings
- Typed frontend IPC layer (`src/ipc`)
- Architecture overview in `docs/architecture.md`
- Vanilla install stage 3: planner and `install_vanilla_client_libraries` command for verified client JAR, libraries, native archives and assets; launch now uses the launcher-owned game directory
- Appearance setting with dark, light, and follow-system colour schemes
- Version manifest parsing for libraries, arguments, asset index, client download and Java version, with rule evaluation
- `inheritsFrom` merging: child libraries first, newest version wins among identical coordinates, parent arguments first
- Microsoft device code login with the Xbox Live, XSTS and Minecraft token chain, plus typed XSTS error codes
- Offline account UUID derived with the Java offline player UUID algorithm, together with player-name validation
- Credential store trait with an atomic file implementation that keeps access tokens off disk
- Resumable download engine: Range continuation, bounded retry with backoff, multi-source fallback with per-provider circuit breaking, progress reporting and cancellation
- Streaming SHA-1 computed while writing, with artifacts published through a `.part` file and an atomic rename
- Fabric, Quilt, Forge and NeoForge metadata parsing with install plans built from injected metadata sources
- `aether-diagnose` crate: launch evidence collection, failure stage determination, local crash classification rules, JVM argument preflight table and a cache signature
- Crash analysis first-layer redaction: launch arguments by position, user names in paths, player names, server addresses, and email detection that ignores `module@version` strings
- Instance creation, atomic record updates and a fault-tolerant scan over the managed instance store
- PCL2 and HMCL detection with import plans that fill only missing files
- Editable instance configuration for memory, Java override, JVM arguments, game arguments and window settings
- `os.version` rules evaluated as regular expressions
- Instance, library and account pages with Java runtime and memory settings blocks
- Official-first mirror fallback: BMCLAPI URLs derived from Mojang endpoints are appended to download plans, and metadata fetches switch to the mirror on transport failures
- Download engine option to reuse already-verified files instead of re-downloading them across shared versions
- `aether-schematic` crate: minimal NBT reader and Litematica block-state decoding with preview size caps, plus a data-version-to-release table
- Schematic management that automatically discovers `.litematic` files in every instance's `schematics` directory and groups them per instance, with metadata showing the originating Minecraft release, import by content, deletion, and cross-instance copy for cross-version use
- Rotatable 3D schematic preview rendered with three.js instanced meshes and lattice sampling above a block budget
- Schematic browser that keeps the `schematics` folder structure: the backend walks subfolders and returns a `/`-separated `relativePath` per file, and only versions that actually contain schematics are listed, as a collapsible per-version tree
- Schematic context menu with copy, cut, paste, delete and multi-select; paste copies or moves files into any folder of any version, renaming on conflict instead of overwriting

- Importing instances from an existing `.minecraft` folder, HMCL, PCL2 or BakaXL installation: the version JSON and its `inheritsFrom` ancestors are copied into the launcher-owned game tree and one managed instance is created per selected version, with the client JAR, libraries and assets fetched afterwards through the existing installer and its BMCLAPI mirror
- PCL2 detection that reads the game folder recorded in `PCL/Setup.ini` (`LaunchFolderSelect`, with `$` resolved against the launcher folder), plus a manual fallback when the bounded search finds nothing
- BakaXL detection by its launcher markers, with the game read from `.minecraft` beside the executable
- Prism Launcher and MultiMC import: instances are read from `mmc-pack.json` and `instance.cfg`, and one managed instance is created per selected base version; loader components are reported but not installed
- One-click launcher detection covering PCL2, HMCL, BakaXL, Prism Launcher and MultiMC in a single bounded pass, with the detected launcher labelled in the import dialog
- Native folder picker in the backend via `tauri-plugin-dialog`, so the import commands never accept a path from the frontend; scanned sources are referenced by a single-use token

### Security

- Crash analysis runs locally first and displays the redacted payload to the user before anything is sent off the machine
- Refresh token persistence is opt-in and disabled by default; the store is atomic but not encrypted, and its protection comes from the application data directory's permissions
- Instance commands accept typed identifiers only, with the storage root directory resolved by the backend rather than supplied by the caller

### Changed

- Home greeting follows the local time of day
- Document language set to `zh-CN`
- Download plans reject unsafe or duplicate destination paths
- `copy_schematic_to_instance` replaced by `paste_schematic`, which covers both same-instance folder moves and cross-version copies; the schematic commands now take a `relativePath` instead of a bare file name

### Fixed

- `.litematic` parsing now follows the layout Litematica actually writes: the name, author, region count and totals live in the `Metadata` compound rather than at the root, the totals are `TAG_Int` (both integer widths are accepted), and a region palette is the `BlockStatePalette` list of block states rather than a name-to-index compound. Every real schematic previously failed with `missing required litematic field RegionCount`, so imports were rejected and no scanned file could be summarised.
- Schematic discovery only read the instance's private `schematics/` directory, so a `.minecraft` full of projections showed up empty: the files live in the game directory (`schematics/` or `versions/<version>/schematics`), which every launcher shares. All three locations are scanned now, each file carries its `origin` and `originVersion`, and the UI groups them by source. The same relative path in two sources stays two files instead of being collapsed.
- A metadata name of `Unnamed` (Litematica's placeholder) is now treated as "no name" and shown as the file name instead of a column of `Unnamed` entries.

### Added

- `import_schematic_folder`: pick a folder and copy every `.litematic` inside it into the selected instance, keeping the subfolder structure, renaming on conflict and reporting the files it had to skip
