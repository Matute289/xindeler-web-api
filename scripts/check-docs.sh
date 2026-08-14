#!/bin/sh
set -eu

documents="README.md CLAUDE.md AGENTS.md .backlog/SPEC.md .backlog/PLAN.md"

# The whole Xindeler ecosystem migrated from *.xindeler.greenmountain.dev to
# *.xindeler.com (see xindeler-web-landing's fix/domain-migration-xindeler-com
# and xindeler-documentation's chore/docs-sync-2026-08-14). A stale reference
# here would document a URL that just 301s. Note: bare `greenmountain.dev` is
# the VPS's own hostname (legitimate, e.g. SSH docs) — only the old
# `xindeler.greenmountain.dev` subdomains are stale.
if grep -n 'xindeler\.greenmountain\.dev' $documents; then
    echo "documentation references the legacy xindeler.greenmountain.dev domain" >&2
    exit 1
fi

grep -F 'WEB_API_BIND_ADDR' README.md >/dev/null
grep -F 'xindeler-auth' README.md >/dev/null
