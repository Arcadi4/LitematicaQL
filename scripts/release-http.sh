#!/usr/bin/env bash
# Shared by the release preflight scripts. Source it, never run it.
#
# The pipeline reads the published feed and packages over HTTPS before it
# publishes. The zone must answer a plain client without a browser challenge.

# fetch_public URL BODY_PATH
#
# Prints the status code. Retries connection errors and 5xx only.
fetch_public() {
  curl -sSL --retry 3 --retry-delay 5 --max-time 60 \
    -o "$2" -w '%{http_code}' "$1"
}

# reject_unavailable URL STATUS
#
# Returns when the status is 200. Prints the reason and exits otherwise.
reject_unavailable() {
  local url="$1" status="$2"
  if [[ "$status" == 200 ]]; then
    return 0
  fi
  echo "Cannot read $url (HTTP $status)." >&2
  exit 1
}
