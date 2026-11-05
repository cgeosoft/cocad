#!/usr/bin/env bash
# Deploys public/ to Cloudflare Pages. Writes the PostHog key into a staged copy,
# so the key stays out of git. Needs wrangler and a Cloudflare login.
#
# Usage: ./deploy.sh [--stage DIR]    --stage only writes the copy to DIR.
set -euo pipefail
cd "$(dirname "$0")"

if [ -f .env ]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

key="${POSTHOG_API_KEY:-}"
case "$key" in
  phc_*) ;;
  *) echo "POSTHOG_API_KEY is not set: analytics stay off and the modal does not show." >&2; key= ;;
esac

if [ "${1:-}" = "--stage" ]; then
  stage=$2
  mkdir -p "$stage"
else
  stage=$(mktemp -d)
  trap 'rm -rf "$stage"' EXIT
fi
cp -R public/. "$stage/"
if [ -n "$key" ]; then
  sed -i "s|YOUR_POSTHOG_PROJECT_API_KEY|$key|" "$stage/analytics.js"
fi
[ "${1:-}" = "--stage" ] && exit 0

wrangler pages deploy "$stage" --project-name "${CLOUDFLARE_PAGES_PROJECT:-scad-viewer}" --branch main
