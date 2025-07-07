#!/bin/bash
# cleanup.sh

echo "🧹 Cleaning up all rustainer resources..."

# Parar todos os processos relacionados
echo "Stopping processes..."
sudo pkill -f "rustainer" 2>/dev/null || true
sudo pkill -f "ip netns exec rustainer" 2>/dev/null || true

# Limpar namespaces de rede
echo "Cleaning network namespaces..."
for ns in $(ip netns list 2>/dev/null | grep rustainer | awk '{print $1}'); do
    echo "Deleting namespace: $ns"
    sudo ip netns delete "$ns" 2>/dev/null || true
done

# Limpar interfaces veth (corrigido para o padrão correto)
echo "Cleaning veth interfaces..."
for veth in $(ip link show 2>/dev/null | grep -E "(veth.*[ch])" | awk '{print $2}' | cut -d':' -f1 | cut -d'@' -f1); do
    echo "Deleting veth: $veth"
    sudo ip link delete "$veth" 2>/dev/null || true
done

# Limpar bridge
echo "Cleaning bridge..."
sudo ip link set rustainer0 down 2>/dev/null || true
sudo ip link delete rustainer0 2>/dev/null || true

# Limpar regras do iptables (mais específico)
echo "Cleaning iptables rules..."
sudo iptables -t nat -D POSTROUTING -s 172.18.0.0/16 ! -o rustainer0 -j MASQUERADE 2>/dev/null || true
sudo iptables -D FORWARD -i rustainer0 -o rustainer0 -j ACCEPT 2>/dev/null || true
sudo iptables -t nat -F POSTROUTING 2>/dev/null || true
sudo iptables -t nat -F PREROUTING 2>/dev/null || true
sudo iptables -F FORWARD 2>/dev/null || true

# Limpar containers
echo "Cleaning containers..."
sudo rm -rf ./containers/rustainer_* 2>/dev/null || true

# Verificação final
echo "🔍 Final verification..."
echo "Namespaces: $(ip netns list 2>/dev/null | grep rustainer | wc -l)"
echo "Veth interfaces: $(ip link show 2>/dev/null | grep -E "(veth.*[ch])" | wc -l)"
echo "Containers: $(ls -la containers/ 2>/dev/null | grep rustainer | wc -l)"

echo "✅ Cleanup completed!"