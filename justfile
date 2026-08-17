# Aircast Flasher — task runner
# Run `just` to list recipes.

_default:
    @just --list

# Run the app in development (Tauri + Vite)
[group('dev')]
dev:
    npm run tauri dev

# Build the frontend bundle
[group('build')]
build:
    npm run build

# Build the desktop app for the current platform
[group('build')]
bundle:
    npm run tauri build

# Rust + frontend unit tests
[group('test')]
test:
    cargo test --workspace
    npm run test:unit
    npm run test:scripts

# End-to-end tests against the real app (WebdriverIO, embedded Tauri driver)
[group('test')]
e2e:
    npm run build
    cargo build --features wdio --manifest-path src-tauri/Cargo.toml
    npx tsc -p e2e/tsconfig.json
    npx wdio run ./wdio.conf.ts

# Everything CI runs
[group('test')]
check: test e2e

# Create and push a release tag.
#   just release <patch|minor|major|dev|staging>
[group('release')]
release TYPE:
    ./scripts/release.sh {{TYPE}}
