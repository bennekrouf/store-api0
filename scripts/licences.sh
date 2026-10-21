#!/usr/bin/env bash
# scripts/licences.sh — see an email's licences, pick one and change it by hand.
#
# Lists every licence of an email (live or ended, Stripe, granted or free),
# then for the one picked: move it to another plan, end it now, give back its
# credits, extend it, or bring an ended one back. Nothing is emailed.
#
#   scripts/licences.sh                 reads DATABASE_URL from /opt/api0/store.env
#   STORE_ENV=./store.env scripts/licences.sh
#
# A licence paid through Stripe is Stripe's to change: its plan and dates are
# overwritten by the next Stripe event, so for those this only gives back
# credits; cancel or change them in the Stripe dashboard. Granted licences
# (scripts/grant-licence.sh) and free allowances are this database's alone.

set -euo pipefail

STORE_ENV="${STORE_ENV:-/opt/api0/store.env}"

die() { echo "error: $*" >&2; exit 1; }

if [[ -z "${DATABASE_URL:-}" ]]; then
  [[ -r "$STORE_ENV" ]] || die "no DATABASE_URL, and $STORE_ENV can't be read (set STORE_ENV)"
  set -a; # shellcheck disable=SC1090
  . "$STORE_ENV"; set +a
fi
[[ -n "${DATABASE_URL:-}" ]] || die "DATABASE_URL is not set in $STORE_ENV"
command -v psql >/dev/null || die "psql is not installed"

# Values go in as psql variables, never pasted into the SQL, so nothing typed
# here can change the statement.
query() { psql "$DATABASE_URL" -X -A -t -q -v ON_ERROR_STOP=1 "$@"; }

ask() { # ask <prompt> <default> → answer on stdout
  local answer
  read -r -p "$1${2:+ [$2]}: " answer
  echo "${answer:-$2}"
}

confirm() { [[ "$(ask "$1 (y/N)" "N")" =~ ^[yY] ]]; }

echo "Licences: see one, change it"
echo

# ── Whose ─────────────────────────────────────────────────────────────────────
while :; do
  EMAIL=$(ask "Email address" "")
  EMAIL=$(echo "$EMAIL" | tr '[:upper:]' '[:lower:]' | xargs)
  [[ "$EMAIL" =~ ^[^[:space:]@]+@[^[:space:]@]+\.[^[:space:]@]+$ ]] && break
  echo "  That doesn't look like an email address."
done

# One line per licence, newest first: licence|kind|plan|apps|state|until|left|granted
ROWS=()
while IFS= read -r row; do [[ -n "$row" ]] && ROWS+=("$row"); done < <(query -F '|' -v email="$EMAIL" <<'SQL'
SELECT s.licence,
       CASE WHEN s.is_free THEN 'free' WHEN s.stripe_customer_id = '' THEN 'granted' ELSE 'stripe' END,
       format('%s (%s)', p.name, p.id),
       CASE WHEN p.products = '{}' THEN 'AI only' WHEN '*' = ANY(p.products) THEN 'every app'
            ELSE array_to_string(p.products, ', ') END,
       CASE WHEN s.status IN ('active', 'trialing') AND s.period_end > NOW()
            THEN CASE WHEN s.cancel_at_period_end THEN 'live, cancelled, ends' ELSE 'live until' END
            WHEN s.status IN ('active', 'trialing') THEN 'ended on'
            ELSE format('ended (%s) on', s.status) END,
       to_char(s.period_end, 'YYYY-MM-DD'),
       GREATEST(s.credits_granted - s.credits_used, 0),
       s.credits_granted
FROM subscriptions s JOIN plans p ON p.id = s.plan_id
WHERE s.email = :'email' AND s.licence IS NOT NULL
ORDER BY s.created_at DESC;
SQL
)
(( ${#ROWS[@]} > 0 )) || { echo "  $EMAIL has no licence. Make one with scripts/grant-licence.sh."; exit 0; }

echo
echo "$EMAIL has:"
for i in "${!ROWS[@]}"; do
  IFS='|' read -r lic kind plan apps state until left granted <<<"${ROWS[$i]}"
  printf '  %d) %s  %-8s %-18s Pro in: %-12s %s %s, %s of %s credits left\n' \
    $((i + 1)) "$lic" "$kind" "$plan" "$apps" "$state" "$until" "$left" "$granted"
done

# ── Which ─────────────────────────────────────────────────────────────────────
while :; do
  PICK=$(ask "Which one" "1")
  [[ "$PICK" =~ ^[0-9]+$ ]] && (( PICK >= 1 && PICK <= ${#ROWS[@]} )) && break
  echo "  Enter a number from 1 to ${#ROWS[@]}."
done
IFS='|' read -r LICENCE KIND PLAN APPS STATE UNTIL LEFT GRANTED <<<"${ROWS[$((PICK - 1))]}"
echo
echo "  $LICENCE — $KIND, $PLAN, Pro in: $APPS, $STATE $UNTIL, $LEFT of $GRANTED credits left"

# ── What ──────────────────────────────────────────────────────────────────────
echo
if [[ "$KIND" == "stripe" ]]; then
  echo "Paid through Stripe: its plan and dates follow Stripe. Cancel or change it in the Stripe dashboard."
  echo "  c) give back its credits (used this period → 0)"
  echo "  q) nothing"
  ACTIONS="c q"
else
  echo "  p) move it to another plan (the same licence number keeps working)"
  echo "  e) end it now"
  echo "  x) extend it, or bring it back if it has ended"
  echo "  c) give back its credits (used → 0), or set a new total"
  echo "  q) nothing"
  ACTIONS="p e x c q"
fi
while :; do
  ACTION=$(ask "Do" "q")
  [[ " $ACTIONS " == *" $ACTION "* ]] && break
  echo "  Pick one of: $ACTIONS"
done

case "$ACTION" in
  q)
    echo "Nothing done."; exit 0 ;;

  p)
    echo
    echo "Plans:"
    query <<'SQL' | sed 's/^/  /'
SELECT format('%-10s %-12s Pro in: %-28s %s credits/month%s', id, name,
              CASE WHEN products = '{}' THEN 'AI only' WHEN '*' = ANY(products) THEN 'every app'
                   ELSE array_to_string(products, ', ') END,
              monthly_credits, CASE WHEN active THEN '' ELSE '  (not on sale)' END)
FROM plans ORDER BY sort_order, id;
SQL
    while :; do
      NEW_PLAN=$(ask "Plan id" "")
      FOUND=$(query -v plan="$NEW_PLAN" <<'SQL'
SELECT 1 FROM plans WHERE id = :'plan';
SQL
)
      [[ -n "$FOUND" ]] && break
      echo "  No plan '$NEW_PLAN'."
    done
    # A free allowance moved to a plan with apps stops being the free one, as
    # when its email subscribes: the store would otherwise renew it as free.
    confirm "Move $LICENCE to $NEW_PLAN?" || { echo "Nothing done."; exit 0; }
    query -v lic="$LICENCE" -v plan="$NEW_PLAN" <<'SQL'
UPDATE subscriptions s
   SET plan_id = :'plan',
       is_free = s.is_free AND (SELECT products = '{}' FROM plans WHERE id = :'plan'),
       updated_at = NOW()
 WHERE s.licence = :'lic';
SQL
    ;;

  e)
    confirm "End $LICENCE now? The apps stop accepting it at their next check" || { echo "Nothing done."; exit 0; }
    query -v lic="$LICENCE" <<'SQL'
UPDATE subscriptions SET period_end = NOW(), updated_at = NOW() WHERE licence = :'lic';
SQL
    ;;

  x)
    while :; do
      DURATION=$(ask "Until when: a number of months from today, or 'forever'" "1")
      if [[ "$DURATION" == "forever" ]]; then
        NEW_UNTIL="2099-12-31"; break
      elif [[ "$DURATION" =~ ^[1-9][0-9]*$ ]] && (( DURATION <= 1200 )); then
        NEW_UNTIL=$(query -v m="$DURATION" <<'SQL'
SELECT to_char(NOW() + make_interval(months => :'m'::int), 'YYYY-MM-DD');
SQL
)
        break
      fi
      echo "  Enter a whole number of months (1–1200), or forever."
    done
    confirm "Make $LICENCE live until $NEW_UNTIL?" || { echo "Nothing done."; exit 0; }
    query -v lic="$LICENCE" -v until="$NEW_UNTIL" <<'SQL'
UPDATE subscriptions
   SET status = 'active', cancel_at_period_end = FALSE,
       period_end = (:'until')::date + 1, updated_at = NOW()
 WHERE licence = :'lic';
SQL
    ;;

  c)
    if [[ "$KIND" == "stripe" ]]; then
      NEW_TOTAL=$GRANTED
    else
      while :; do
        NEW_TOTAL=$(ask "Credits in total for this period" "$GRANTED")
        [[ "$NEW_TOTAL" =~ ^[0-9]+$ ]] && break
        echo "  Enter a whole number."
      done
    fi
    confirm "Give $LICENCE $NEW_TOTAL credits, none used?" || { echo "Nothing done."; exit 0; }
    query -v lic="$LICENCE" -v total="$NEW_TOTAL" <<'SQL'
UPDATE subscriptions SET credits_granted = (:'total')::bigint, credits_used = 0, updated_at = NOW()
 WHERE licence = :'lic';
SQL
    ;;
esac

echo
echo "Done. $LICENCE is now:"
query -v lic="$LICENCE" <<'SQL' | sed 's/^/  /'
SELECT format('%s (%s), Pro in: %s, %s %s, %s of %s credits left', p.name, p.id,
              CASE WHEN p.products = '{}' THEN 'AI only' WHEN '*' = ANY(p.products) THEN 'every app'
                   ELSE array_to_string(p.products, ', ') END,
              CASE WHEN s.status IN ('active', 'trialing') AND s.period_end > NOW() THEN 'live until' ELSE 'ended on' END,
              to_char(s.period_end, 'YYYY-MM-DD'),
              GREATEST(s.credits_granted - s.credits_used, 0), s.credits_granted)
FROM subscriptions s JOIN plans p ON p.id = s.plan_id WHERE s.licence = :'lic';
SQL
echo
echo "Check what the apps see with:"
echo "  curl -s https://gateway.api0.ai/v1/licence -H \"Authorization: Bearer $LICENCE\""
