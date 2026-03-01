#!/usr/bin/env bash

set -euo pipefail

IMAGE_NAME="${IMAGE_NAME:-refreshmint-scrape-ci-repro}"
CONTAINER_NAME="${CONTAINER_NAME:-refreshmint-scrape-ci-repro}"
DOCKERFILE="${DOCKERFILE:-docker/scrape-ci-repro.Dockerfile}"
PLATFORM="${PLATFORM:-}"

BUILD_ARGS=()
if [[ -n "$PLATFORM" ]]; then
  BUILD_ARGS+=(--platform "$PLATFORM")
fi

podman build "${BUILD_ARGS[@]}" -f "$DOCKERFILE" -t "$IMAGE_NAME" .

if podman container exists "$CONTAINER_NAME"; then
  echo "Container already exists: $CONTAINER_NAME" >&2
  echo "Run it with: podman start -ai $CONTAINER_NAME" >&2
  exit 0
fi

podman create --name "$CONTAINER_NAME" "$IMAGE_NAME"
echo "Created container: $CONTAINER_NAME"
echo "Start it with: podman start -ai $CONTAINER_NAME"
