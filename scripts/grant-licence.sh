#!/usr/bin/env bash
# scripts/grant-licence.sh — give someone a licence by hand, without Stripe.
#
# For the owner, a reviewer, a partner, a refund made good: asks for the email,
# which plan (or every app), for how long and how many AI credits, then makes
# the subscription and prints its licence number. Nothing is emailed: send the
# number yourself.
#
#   scripts/grant-licence.sh                 reads DATABASE_URL from /opt/api0/store.env
#   STORE_ENV=./store.env scripts/grant-licence.sh
#
# A granted licence is an ordinary subscription row (payment::subscription)
# with no Stripe customer behind it: it is active until the end date and its
# credits are a total for the whole period — they never refill. Top them up with
#   UPDATE subscriptions SET credits_used = 0 WHERE licence = '<licence>';
# and end it early with
#   UPDATE subscriptions SET period_end = NOW() WHERE licence = '<licence>';

set -euo pipefail

STORE_ENV="${STORE_ENV:-/opt/api0/store.env}"
ALPHABET='ABCDEFGHJKLMNPQRSTUVWXYZ23456789'

die() { echo "error: $*" >&2; exit 1; }

if [[ -z "${DATABASE_URL:-}" ]]; then
  [[ -r "$STORE_ENV" ]] || die "no DATABASE_URL, and $STORE_ENV can't be read (set STORE_ENV)"
  set -a; # shellcheck disable=SC1090
  . "$STORE_ENV"; set +a
fi
[[ -n "${DATABASE_URL:-}" ]] || die "DATABASE_URL is not set in $STORE_ENV"
command -v psql >/dev/null || die "psql is not installed"
command -v python3 >/dev/null || die "python3 is needed to make the licence number"

# One value out of the database. Values go in as psql variables, never pasted
# into the SQL, so nothing typed here can change the statement.
query() { psql "$DATABASE_URL" -X -A -t -q -v ON_ERROR_STOP=1 "$@"; }

ask() { # ask <prompt> <default> → answer on stdout
  local answer
  read -r -p "$1${2:+ [$2]}: " answer
  echo "${answer:-$2}"
}

echo "Grant a licence (no Stripe, nothing emailed)"
echo

# ── Who ───────────────────────────────────────────────────────────────────────
while :; do
  EMAIL=$(ask "Email address" "")
  EMAIL=$(echo "$EMAIL" | tr '[:upper:]' '[:lower:]' | xargs)
  [[ "$EMAIL" =~ ^[^[:space:]@]+@[^[:space:]@]+\.[^[:space:]@]+$ ]] && break
  echo "  That doesn't look like an email address."
done

EXISTING=$(query -v email="$EMAIL" <<'SQL'
SELECT string_agg(format('%s (%s, %s until %s)', p.name, s.licence, s.status, to_char(s.period_end, 'YYYY-MM-DD')), E'\n  ')
FROM subscriptions s JOIN plans p ON p.id = s.plan_id
WHERE s.email = :'email' AND s.licence IS NOT NULL AND s.status IN ('active', 'trialing') AND s.period_end > NOW();
SQL
)
if [[ -n "$EXISTING" ]]; then
  echo "  $EMAIL already has:"
  echo "  $EXISTING"
  [[ "$(ask "  Grant another licence anyway? (y/N)" "N")" =~ ^[yY] ]] || { echo "Nothing done."; exit 0; }
fi

# ── What ──────────────────────────────────────────────────────────────────────
echo
echo "Plans (any of them can be granted, on sale or not):"
query <<'SQL' | sed 's/^/  /'
SELECT format('%-10s %-12s apps: %-30s %s credits/month%s', id, name,
              CASE WHEN products = '{}' THEN '(none: AI only)' ELSE array_to_string(products, ', ') END,
              monthly_credits, CASE WHEN active THEN '' ELSE '  (not on sale)' END)
FROM plans ORDER BY sort_order, id;
SQL
echo "  owner      Owner        apps: * (every app)                 made if missing, never on sale"
while :; do
  PLAN=$(ask "Plan id" "owner")
  [[ "$PLAN" == "owner" ]] && break
  FOUND=$(query -v plan="$PLAN" <<'SQL'
SELECT 1 FROM plans WHERE id = :'plan';
SQL
)
  [[ -n "$FOUND" ]] && break
  echo "  No plan '$PLAN'."
done

PLAN_NAME=Owner
PLAN_APPS='every app'
DEFAULT_CREDITS=100000
if [[ "$PLAN" != "owner" ]]; then
  IFS='|' read -r PLAN_NAME PLAN_APPS PLAN_MONTHLY < <(query -v plan="$PLAN" <<'SQL'
SELECT name,
       CASE WHEN products = '{}' THEN 'no Pro (AI only)' WHEN '*' = ANY(products) THEN 'every app'
            ELSE array_to_string(products, ', ') END,
       monthly_credits
FROM plans WHERE id = :'plan';
SQL
)
fi

# ── How long ──────────────────────────────────────────────────────────────────
while :; do
  DURATION=$(ask "How long: a number of months, or 'forever'" "12")
  if [[ "$DURATION" == "forever" ]]; then
    UNTIL="2099-12-31"; MONTHS=0; break
  elif [[ "$DURATION" =~ ^[1-9][0-9]*$ ]] && (( DURATION <= 1200 )); then
    MONTHS=$DURATION
    UNTIL=$(query -v m="$MONTHS" <<'SQL'
SELECT to_char(NOW() + make_interval(months => :'m'::int), 'YYYY-MM-DD');
SQL
)
    break
  fi
  echo "  Enter a whole number of months (1–1200), or forever."
done

# ── AI credits ────────────────────────────────────────────────────────────────
if [[ "$PLAN" != "owner" ]]; then
  # The plan's monthly allowance for each month granted, since nothing refills.
  if (( MONTHS > 0 )); then DEFAULT_CREDITS=$(( PLAN_MONTHLY * MONTHS )); else DEFAULT_CREDITS=$(( PLAN_MONTHLY * 120 )); fi
fi
while :; do
  CREDITS=$(ask "AI credits for the whole period (they don't refill)" "$DEFAULT_CREDITS")
  [[ "$CREDITS" =~ ^[0-9]+$ ]] && break
  echo "  Enter a whole number."
done

# ── Confirm ───────────────────────────────────────────────────────────────────
echo
echo "About to grant:"
echo "  email    $EMAIL"
echo "  plan     $PLAN_NAME ($PLAN) — Pro in: $PLAN_APPS"
echo "  until    $UNTIL"
echo "  credits  $CREDITS AI credits in total"
[[ "$(ask "Create it? (y/N)" "N")" =~ ^[yY] ]] || { echo "Nothing done."; exit 0; }

LICENCE=$(python3 -c "import secrets,sys
a=sys.argv[1]; s=''.join(secrets.choice(a) for _ in range(20))
print('-'.join(s[i:i+5] for i in range(0, 20, 5)))" "$ALPHABET")

query -v email="$EMAIL" -v plan="$PLAN" -v until="$UNTIL" -v credits="$CREDITS" -v lic="$LICENCE" <<'SQL'
BEGIN;
INSERT INTO plans (id, name, description, price_cents, currency, monthly_credits, products, active, sort_order)
SELECT 'owner', 'Owner', 'Not for sale: every app in Pro, granted by hand.', 0, 'eur', 0, '{*}', FALSE, 99
WHERE :'plan' = 'owner'
ON CONFLICT (id) DO NOTHING;
INSERT INTO subscriptions (id, email, plan_id, stripe_customer_id, status, cancel_at_period_end,
                           period_start, period_end, credits_granted, credits_used,
                           licence, licence_emailed_at, is_free)
VALUES ('grant_' || md5(:'lic'), :'email', :'plan', '', 'active', FALSE,
        NOW(), (:'until')::date + 1, (:'credits')::bigint, 0,
        :'lic', NOW(), FALSE);
COMMIT;
SQL

echo
echo "Done. Licence for $EMAIL:"
echo
echo "    $LICENCE"
echo
echo "Paste it into the app's licence window. Check it with:"
echo "  curl -s https://gateway.api0.ai/v1/licence -H \"Authorization: Bearer $LICENCE\""
