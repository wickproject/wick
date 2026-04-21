#!/bin/bash
# Wick: WireGuard tunnel setup for routing CEF requests through a
# residential IP.
#
# Usage:
#   On the cloud server:  ./setup-wireguard.sh server <client-public-key>
#   On the client laptop: ./setup-wireguard.sh client <server-ip> <server-public-key>
#
# Generates keys, creates configs, starts the tunnel.
# Requires: wireguard-tools, root/sudo access.

set -euo pipefail

WG_INTERFACE="wg-wick"
SERVER_PORT=51820
SERVER_ADDR="10.99.0.1/24"
CLIENT_ADDR="10.99.0.2/24"
NETWORK="10.99.0.0/24"

generate_keys() {
    local name="$1"
    local key_dir="/etc/wireguard"
    mkdir -p "$key_dir"

    if [ ! -f "$key_dir/${name}-private.key" ]; then
        wg genkey > "$key_dir/${name}-private.key"
        chmod 600 "$key_dir/${name}-private.key"
        wg pubkey < "$key_dir/${name}-private.key" > "$key_dir/${name}-public.key"
        echo "Generated keys for $name"
    else
        echo "Keys already exist for $name"
    fi

    PRIVATE_KEY=$(cat "$key_dir/${name}-private.key")
    PUBLIC_KEY=$(cat "$key_dir/${name}-public.key")
}

setup_server() {
    local client_pubkey="${1:?Usage: setup-wireguard.sh server <client-public-key>}"

    generate_keys "server"

    # Detect main network interface
    local main_iface
    main_iface=$(ip route show default | awk '{print $5}' | head -1)

    cat > "/etc/wireguard/${WG_INTERFACE}.conf" << EOF
[Interface]
PrivateKey = ${PRIVATE_KEY}
Address = ${SERVER_ADDR}
ListenPort = ${SERVER_PORT}

[Peer]
# Client (residential IP exit point)
PublicKey = ${client_pubkey}
AllowedIPs = ${NETWORK}
EOF

    # Enable IP forwarding
    sysctl -w net.ipv4.ip_forward=1 > /dev/null
    echo "net.ipv4.ip_forward=1" >> /etc/sysctl.d/99-wick.conf 2>/dev/null || true

    # Start the interface
    wg-quick down "$WG_INTERFACE" 2>/dev/null || true
    wg-quick up "$WG_INTERFACE"

    # Add policy routing: traffic from the WireGuard subnet exits via the tunnel
    ip rule add from 10.99.0.2 table 200 2>/dev/null || true
    ip route add default via 10.99.0.2 table 200 2>/dev/null || true

    echo ""
    echo "=== Server setup complete ==="
    echo "Server public key: ${PUBLIC_KEY}"
    echo "Server endpoint:   $(curl -s https://httpbin.org/ip | grep -o '"origin": "[^"]*"' | cut -d'"' -f4):${SERVER_PORT}"
    echo "WireGuard interface: ${WG_INTERFACE}"
    echo ""
    echo "Next: run on the client machine:"
    echo "  ./setup-wireguard.sh client <server-ip> ${PUBLIC_KEY}"
}

setup_client() {
    local server_ip="${1:?Usage: setup-wireguard.sh client <server-ip> <server-public-key>}"
    local server_pubkey="${2:?Usage: setup-wireguard.sh client <server-ip> <server-public-key>}"

    generate_keys "client"

    # Detect main network interface for NAT masquerade
    local main_iface
    if [[ "$(uname)" == "Darwin" ]]; then
        main_iface=$(route -n get default 2>/dev/null | grep interface | awk '{print $2}')
    else
        main_iface=$(ip route show default | awk '{print $5}' | head -1)
    fi

    cat > "/etc/wireguard/${WG_INTERFACE}.conf" << EOF
[Interface]
PrivateKey = ${PRIVATE_KEY}
Address = ${CLIENT_ADDR}
# NAT masquerade: tunnel traffic exits via residential connection
PostUp = iptables -t nat -A POSTROUTING -s ${NETWORK} -o ${main_iface} -j MASQUERADE; sysctl -w net.ipv4.ip_forward=1
PostDown = iptables -t nat -D POSTROUTING -s ${NETWORK} -o ${main_iface} -j MASQUERADE

[Peer]
PublicKey = ${server_pubkey}
Endpoint = ${server_ip}:${SERVER_PORT}
AllowedIPs = 10.99.0.0/24
PersistentKeepalive = 25
EOF

    # Start the interface
    wg-quick down "$WG_INTERFACE" 2>/dev/null || true
    wg-quick up "$WG_INTERFACE"

    echo ""
    echo "=== Client setup complete ==="
    echo "Client public key: ${PUBLIC_KEY}"
    echo "Tunnel active: ${WG_INTERFACE}"
    echo ""
    echo "Give this key to the server operator:"
    echo "  ${PUBLIC_KEY}"
    echo ""
    echo "Test from the server:"
    echo "  curl --interface 10.99.0.1 https://httpbin.org/ip"
    echo "  # Should show your residential IP"
}

show_status() {
    echo "=== WireGuard Status ==="
    wg show "$WG_INTERFACE" 2>/dev/null || echo "Interface $WG_INTERFACE not active"
    echo ""
    if ip addr show "$WG_INTERFACE" &>/dev/null; then
        echo "Interface IP:"
        ip addr show "$WG_INTERFACE" | grep inet
    fi
}

case "${1:-help}" in
    server)
        shift
        setup_server "$@"
        ;;
    client)
        shift
        setup_client "$@"
        ;;
    status)
        show_status
        ;;
    keys)
        generate_keys "${2:-wick}"
        echo "Public key: ${PUBLIC_KEY}"
        ;;
    *)
        echo "Wick Pro: WireGuard tunnel for residential IP routing"
        echo ""
        echo "Usage:"
        echo "  $0 server <client-public-key>    # Run on cloud server"
        echo "  $0 client <server-ip> <server-pk> # Run on client machine"
        echo "  $0 status                          # Show tunnel status"
        echo "  $0 keys [name]                     # Generate a keypair"
        ;;
esac
