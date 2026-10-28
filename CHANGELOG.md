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

### Changed

- **Instances now read the user's own `.minecraft` instead of an app-private copy.** Previously every create/import wrote into `%APPDATA%\app.aetherlauncher.desktop\game` and gave each instance an empty `minecraft/` folder, so a machine whose real game directory held 1652 `.litematic` files showed three of them, and launching meant downloading tens of gigabytes a second time. A record now carries a `game_directory` field pointing at the real `.minecraft`, resolved by the backend (the frontend never passes a path) and validated as an existing, non-symlinked directory. Several instances may share one game directory, exactly like PCL2/HMCL/Prism.
- Creating an instance asks for the game directory with a native folder picker; cancelling returns `null` rather than an error.
- `install_vanilla_client_libraries` takes the instance ID and downloads into that instance's game directory. The "download and complete" button in the create dialog now only caches the official version JSON, because the game directory is not known until the instance exists; installing happens from the instances page afterwards.
- `create_instance` no longer creates an empty `minecraft/` folder. Records written before this change have no `game_directory` and fall back to `root/minecraft`, so existing instances keep working.

### Fixed

- Importing a version whose `versions/<id>/<id>.json` already exists used to skip silently. It is now reported as `DuplicateName` so the UI can say the version is already installed; the existing file is still never overwritten. A parent version that is already present (the vanilla JSON in a shared `.minecraft`) is not treated as a conflict.
- Existing instances created before `game_directory` existed kept an empty `minecraft/` folder, so they showed none of the schematics that were really on disk. `locate_game_directory_for_version` now finds the `.minecraft` that actually contains the instance's version — using launcher configuration as a hint and requiring the version folder to exist — and schematic discovery, launch and download all resolve through the same function. The lookup is read-only and never rewrites the user's record.
- The launch coordinator registry is keyed by game directory, so instances bound to different `.minecraft` folders no longer share one coordinator and stopping a game scans the game directories that were actually started.
- Instance records written before `game_directory` existed showed no schematics at all, because nothing named a game directory. `list_schematics` now reports how many instances are in that state and the schematics page offers a "set game directory" action: one folder picker, then every unfinished record is rewritten in a batch. Each `instance.json` is backed up to `instances/.backup-<timestamp>/` before anything is changed, and instances that already have one are left alone.

### Removed

- The launcher-configuration fallback that looked for PCL2's `Setup.ini` to guess a game directory. Aether is a standalone launcher and must not depend on another launcher being installed; the game directory comes from its own instance records only.

### Added

- Delete an instance from the list. Only the instance record is removed; `versions/`, `saves/`, `mods/` and `schematics/` in the user's .minecraft are never touched, and a test asserts the game directory is unchanged.
- Instances are grouped by loader (原版游戏 / Forge / NeoForge / Fabric / Quilt / LiteLoader / 整合包), derived from the version name.
### Performance

- `list_schematics` scans each game directory once instead of once per instance. With 134 instances sharing one .minecraft the old code walked the same tree 134 times; measured 15.1 s → 119 ms on the same store.
- The instances page mounts a launch panel only for the selected instance instead of one per row (134 cards before), and starts each panel with the instance name rather than its UUID.
- Loader groups on the instances page can be collapsed.
### Performance

- Reading a `.litematic`'s metadata inflates only as far as the root scalars and then stops. Schematics compress at ~55:1, so the old full inflate cost 3.6 s for 1649 files; the whole listing is now 1.40 s instead of 3.99 s.
### Fixed

- Instance rows no longer collapse: the row is a flex container again after the clickable wrapper broke the four-column grid, and the meta line truncates instead of wrapping over three lines.
- Instance status is derived from the disk (`versions/<version>/<version>.jar` present) instead of the `incomplete` value written at creation, so installed instances no longer show 待补齐 forever.
### Fixed

- The schematic scan cache never hit: its key included the instance root, which is unique per instance, so all 134 instances rescanned the same `.minecraft`. The key is the game directory now; a regression test pins it.