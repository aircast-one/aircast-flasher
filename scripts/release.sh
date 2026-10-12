#!/usr/bin/env bash
set -euo pipefail

TYPE="${1:?Usage: $0 <patch|minor|major>}"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${GREEN}▶${NC} $1"; }
log_warn() { echo -e "${YELLOW}▶${NC} $1" >&2; }

if [ -n "$(git status --porcelain)" ]; then
    log_warn "Working tree is not clean — commit or drop changes before releasing."
    exit 1
fi

log_info "Fetching tags from origin..."
git fetch --tags --quiet origin

get_current_version() {
    git tag -l 'v[0-9]*.[0-9]*.[0-9]*' --sort=-v:refname | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | head -1 || echo "v0.0.0"
}

tag_exists() {
    git rev-parse "$1" >/dev/null 2>&1
}

find_next_version() {
    local version="$1"
    while tag_exists "$version"; do
        log_warn "Tag $version already exists, trying next..."
        IFS='.' read -r major minor patch <<< "${version#v}"
        version="v${major}.${minor}.$((patch + 1))"
    done
    echo "$version"
}

parse_version() {
    local version="${1#v}"
    version="${version%%-*}"
    echo "$version"
}

CURRENT=$(get_current_version)
BASE=$(parse_version "$CURRENT")
IFS='.' read -r MAJOR MINOR PATCH <<< "$BASE"

log_info "Current version: $CURRENT"

case "$TYPE" in
    patch)
        NEW_VERSION="v${MAJOR}.${MINOR}.$((PATCH + 1))"
        NEW_VERSION=$(find_next_version "$NEW_VERSION")
        MESSAGE="Release $NEW_VERSION"
        ;;
    minor)
        NEW_VERSION="v${MAJOR}.$((MINOR + 1)).0"
        NEW_VERSION=$(find_next_version "$NEW_VERSION")
        MESSAGE="Release $NEW_VERSION"
        ;;
    major)
        NEW_VERSION="v$((MAJOR + 1)).0.0"
        NEW_VERSION=$(find_next_version "$NEW_VERSION")
        MESSAGE="Release $NEW_VERSION"
        ;;
    *)
        echo "Unknown release type: $TYPE"
        echo "Valid types: patch, minor, major"
        exit 1
        ;;
esac

log_info "New version: $NEW_VERSION"

log_info "Writing version into package.json, tauri.conf.json, Cargo.toml..."
node scripts/sync-version.mjs "${NEW_VERSION#v}"
cargo update --workspace --quiet
git add package.json src-tauri/tauri.conf.json Cargo.toml Cargo.lock
if ! git diff --cached --quiet; then
    git commit -m "chore: bump version to ${NEW_VERSION#v}"
    git push origin HEAD
fi

log_info "Creating tag..."

git tag -a "$NEW_VERSION" -m "$MESSAGE"
git push origin "$NEW_VERSION"

log_info "Released $NEW_VERSION"
