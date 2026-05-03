#!/usr/bin/env bash
# Build a SOCKS5 URL for one of the supported residential proxy providers.
#
#   Usage: proxy-providers.sh --provider=<name> [--country=<cc>] [--session-len=<sec>]
#
# Outputs a single line on stdout:
#   socks5://LOGIN:PASSWORD@HOST:PORT
#
# Reads credentials from the env (intentionally — never on the command line,
# never to disk). Set whichever variables match your provider:
#
#   Oxylabs       OXY_USER, OXY_PASS
#   Bright Data   BRD_CUSTOMER_ID, BRD_ZONE, BRD_PASSWORD
#   IPRoyal       IPR_USER, IPR_PASSWORD
#   SOAX          SOAX_PACKAGE_ID, SOAX_PASSWORD
#   PacketStream  PS_USER, PS_AUTH_KEY
#
# Username-format conventions and country-code → name map are lifted from
# getlantern/lantern-cloud's pingercommon.proxyEnvForCountry — the only
# piece of pinger Wick's bench actually needs. SOCKS5 ports are per each
# provider's documented residential endpoints (≠ their HTTP-proxy ports
# in some cases — Bright Data and PacketStream notably).

set -euo pipefail

PROVIDER=""
COUNTRY="us"
SESSION_LEN=300  # seconds; only used by SOAX and IPRoyal session strings
for arg in "$@"; do
    case $arg in
        --provider=*) PROVIDER="${arg#*=}" ;;
        --country=*)  COUNTRY="${arg#*=}" ;;
        --session-len=*) SESSION_LEN="${arg#*=}" ;;
        *) echo "unknown arg: $arg" >&2; exit 2 ;;
    esac
done

if [[ -z "$PROVIDER" ]]; then
    echo "ERROR: --provider= is required (oxylabs|brightdata|iproyal|soax|packetstream)" >&2
    exit 2
fi

# Lowercase and validate country code (ISO 3166-1 alpha-2 expected).
CC="$(echo "$COUNTRY" | tr '[:upper:]' '[:lower:]')"

# Fresh n-digit session ID per call. Concatenating three $RANDOM values
# gives ~45 bits of entropy — plenty for keeping per-session IPs distinct
# across the bench's modest fetch rate. Pure bash so we avoid SIGPIPE
# from /dev/urandom under `set -e`.
session_id() {
    local n=${1:-10}
    local s
    s="$(printf '%05d%05d%05d' "$RANDOM" "$RANDOM" "$RANDOM")"
    printf '%s' "${s:0:n}"
}

require() {
    for v in "$@"; do
        if [[ -z "${!v:-}" ]]; then
            echo "ERROR: env var $v is required for provider=$PROVIDER" >&2
            exit 3
        fi
    done
}

# country_name_for cc → spelled-out name (PacketStream-format).
# Lifted directly from pingercommon.countryCodeToName. ISO 3166-1
# alpha-2 input. If the code isn't in the map we uppercase the input
# and let PacketStream try; their parser is permissive.
country_name_for() {
    local cc="$1"
    case "$cc" in
        af) echo "Afghanistan" ;;       al) echo "Albania" ;;
        dz) echo "Algeria" ;;           ao) echo "Angola" ;;
        ar) echo "Argentina" ;;         am) echo "Armenia" ;;
        au) echo "Australia" ;;         at) echo "Austria" ;;
        az) echo "Azerbaijan" ;;        bd) echo "Bangladesh" ;;
        by) echo "Belarus" ;;           be) echo "Belgium" ;;
        bo) echo "Bolivia" ;;           ba) echo "Bosnia and Herzegovina" ;;
        br) echo "Brazil" ;;            bg) echo "Bulgaria" ;;
        kh) echo "Cambodia" ;;          cm) echo "Cameroon" ;;
        ca) echo "Canada" ;;            cl) echo "Chile" ;;
        cn) echo "China" ;;             co) echo "Colombia" ;;
        cr) echo "Costa Rica" ;;        hr) echo "Croatia" ;;
        cu) echo "Cuba" ;;              cy) echo "Cyprus" ;;
        cz) echo "Czech Republic" ;;    dk) echo "Denmark" ;;
        do) echo "Dominican Republic" ;; ec) echo "Ecuador" ;;
        eg) echo "Egypt" ;;             sv) echo "El Salvador" ;;
        ee) echo "Estonia" ;;           et) echo "Ethiopia" ;;
        fi) echo "Finland" ;;           fr) echo "France" ;;
        ge) echo "Georgia" ;;           de) echo "Germany" ;;
        gh) echo "Ghana" ;;             gr) echo "Greece" ;;
        gt) echo "Guatemala" ;;         hn) echo "Honduras" ;;
        hk) echo "Hong Kong" ;;         hu) echo "Hungary" ;;
        in) echo "India" ;;             id) echo "Indonesia" ;;
        ir) echo "Iran" ;;              iq) echo "Iraq" ;;
        ie) echo "Ireland" ;;           il) echo "Israel" ;;
        it) echo "Italy" ;;             jm) echo "Jamaica" ;;
        jp) echo "Japan" ;;             jo) echo "Jordan" ;;
        kz) echo "Kazakhstan" ;;        ke) echo "Kenya" ;;
        kw) echo "Kuwait" ;;            kg) echo "Kyrgyzstan" ;;
        la) echo "Laos" ;;              lv) echo "Latvia" ;;
        lb) echo "Lebanon" ;;           ly) echo "Libya" ;;
        lt) echo "Lithuania" ;;         lu) echo "Luxembourg" ;;
        my) echo "Malaysia" ;;          mx) echo "Mexico" ;;
        md) echo "Moldova" ;;           mn) echo "Mongolia" ;;
        ma) echo "Morocco" ;;           mz) echo "Mozambique" ;;
        mm) echo "Myanmar" ;;           np) echo "Nepal" ;;
        nl) echo "Netherlands" ;;       nz) echo "New Zealand" ;;
        ni) echo "Nicaragua" ;;         ng) echo "Nigeria" ;;
        no) echo "Norway" ;;            om) echo "Oman" ;;
        pk) echo "Pakistan" ;;          pa) echo "Panama" ;;
        py) echo "Paraguay" ;;          pe) echo "Peru" ;;
        ph) echo "Philippines" ;;       pl) echo "Poland" ;;
        pt) echo "Portugal" ;;          qa) echo "Qatar" ;;
        ro) echo "Romania" ;;           ru) echo "Russia" ;;
        sa) echo "Saudi Arabia" ;;      rs) echo "Serbia" ;;
        sg) echo "Singapore" ;;         sk) echo "Slovakia" ;;
        si) echo "Slovenia" ;;          za) echo "South Africa" ;;
        kr) echo "South Korea" ;;       es) echo "Spain" ;;
        lk) echo "Sri Lanka" ;;         se) echo "Sweden" ;;
        ch) echo "Switzerland" ;;       tw) echo "Taiwan" ;;
        tj) echo "Tajikistan" ;;        tz) echo "Tanzania" ;;
        th) echo "Thailand" ;;          tn) echo "Tunisia" ;;
        tr) echo "Turkey" ;;            tm) echo "Turkmenistan" ;;
        ua) echo "Ukraine" ;;           ae) echo "United Arab Emirates" ;;
        gb) echo "United Kingdom" ;;    us) echo "United States" ;;
        uy) echo "Uruguay" ;;           uz) echo "Uzbekistan" ;;
        ve) echo "Venezuela" ;;         vn) echo "Vietnam" ;;
        ye) echo "Yemen" ;;             zm) echo "Zambia" ;;
        zw) echo "Zimbabwe" ;;
        *) echo "$(echo "$cc" | tr '[:lower:]' '[:upper:]')" ;;
    esac
}

case "$PROVIDER" in
    oxylabs)
        require OXY_USER OXY_PASS
        login="customer-${OXY_USER}-cc-${CC}-sessid-$(session_id 10)-sesstime-10"
        echo "socks5://${login}:${OXY_PASS}@pr.oxylabs.io:7777"
        ;;
    brightdata)
        # BD's SOCKS5 endpoint runs on a different port than HTTP (33335).
        # Their docs list 24000 as the SOCKS5 default for the unblocker
        # super-proxy. If your zone uses a different port, override with
        # BRD_PORT in the env.
        require BRD_CUSTOMER_ID BRD_ZONE BRD_PASSWORD
        port="${BRD_PORT:-24000}"
        login="brd-customer-${BRD_CUSTOMER_ID}-zone-${BRD_ZONE}-country-${CC}-session-$(session_id 10)"
        echo "socks5://${login}:${BRD_PASSWORD}@brd.superproxy.io:${port}"
        ;;
    iproyal)
        # IPRoyal puts the rotation params in the password field.
        require IPR_USER IPR_PASSWORD
        password="${IPR_PASSWORD}_country-${CC}_session-$(session_id 8)_lifetime-5m"
        echo "socks5://${IPR_USER}:${password}@geo.iproyal.com:32325"
        ;;
    soax)
        require SOAX_PACKAGE_ID SOAX_PASSWORD
        login="package-${SOAX_PACKAGE_ID}-country-${CC}-sessionid-$(session_id 8)-sessionlength-${SESSION_LEN}"
        echo "socks5://${login}:${SOAX_PASSWORD}@proxy.soax.com:5000"
        ;;
    packetstream)
        # PacketStream wants the spelled-out country name in the password
        # field. SOCKS5 listens on 31114 (HTTP is 31113).
        require PS_USER PS_AUTH_KEY
        country_name=$(country_name_for "$CC")
        password="${PS_AUTH_KEY}_country-${country_name}"
        echo "socks5://${PS_USER}:${password}@proxy.packetstream.io:31114"
        ;;
    *)
        echo "ERROR: unknown provider $PROVIDER (oxylabs|brightdata|iproyal|soax|packetstream)" >&2
        exit 2
        ;;
esac
