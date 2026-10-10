#!/usr/bin/env bash
set -Eeuo pipefail

ROLE="${1:-orchestrator}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROMPT_ROOT="$ROOT/.open365/claude/prompts/team"

case "$ROLE" in
  orchestrator)
    if [[ "${OPEN365_TEAM_MODE:-0}" == "1" ]]; then
      PROMPT_FILE="$PROMPT_ROOT/orchestrator.md"
    else
      PROMPT_FILE="$ROOT/.open365/claude/START_HERE.md"
    fi
    MODEL="${OPEN365_OPUS_MODEL:-opus}"
    EFFORT="${OPEN365_OPUS_EFFORT:-medium}"
    ;;
  word|excel|native-ui|collaboration|ai-provider|security)
    PROMPT_FILE="$PROMPT_ROOT/$ROLE.md"
    MODEL="${OPEN365_ROLE_MODEL:-${OPEN365_TEAM_MODEL:-sonnet}}"
    EFFORT="${OPEN365_ROLE_EFFORT:-${OPEN365_TEAM_EFFORT:-medium}}"
    if [[ "$ROLE" == "security" ]]; then
      MODEL="${OPEN365_SECURITY_MODEL:-opus}"
      EFFORT="${OPEN365_SECURITY_EFFORT:-medium}"
    fi
    ;;
  *)
    printf 'Unknown role: %s\nRoles: orchestrator, word, excel, native-ui, collaboration, ai-provider, security\n' "$ROLE" >&2
    exit 2
    ;;
esac

if [[ ! -f "$PROMPT_FILE" ]]; then
  printf 'Prompt file missing: %s\n' "$PROMPT_FILE" >&2
  exit 1
fi
if ! command -v claude >/dev/null 2>&1; then
  printf 'Claude Code CLI not found on PATH. Install and authenticate Claude Code first.\n' >&2
  exit 127
fi

prompt="$(cat "$PROMPT_FILE")"
cd -- "$ROOT"
exec claude \
  --model "$MODEL" \
  --effort "$EFFORT" \
  --permission-mode "${OPEN365_PERMISSION_MODE:-default}" \
  "$prompt"
