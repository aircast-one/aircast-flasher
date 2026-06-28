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

# Create and push a release tag.
#   just release <patch|minor|major|dev|staging>
[group('release')]
release TYPE:
    ./scripts/release.sh {{TYPE}}
