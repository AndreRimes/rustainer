
for iface in all default rustainer0; do
  echo "Desativando rp_filter para $iface"
  echo 0 | sudo tee /proc/sys/net/ipv4/conf/$iface/rp_filter
done

echo
echo "Verificando valores atuais:"
for iface in all default rustainer0; do
  echo "$iface: $(cat /proc/sys/net/ipv4/conf/$iface/rp_filter)"
done

sudo sysctl -w net.ipv4.conf.rustainer0.rp_filter=0


echo "
net.ipv4.conf.all.rp_filter = 0
net.ipv4.conf.default.rp_filter = 0
net.ipv4.conf.rustainer0.rp_filter = 0
" | sudo tee /etc/sysctl.d/99-rustainer.conf

# Aplicar imediatamente
sudo sysctl -p /etc/sysctl.d/99-rustainer.conf

#!/bin/bash

# Aguarda até a interface existir (timeout de 15s)
for i in {1..15}; do
    if ip link show rustainer0 &>/dev/null; then
        echo "Desativando rp_filter na rustainer0"
        sysctl -w net.ipv4.conf.rustainer0.rp_filter=0
        exit 0
    fi
    sleep 1
done

echo "Interface rustainer0 não encontrada após 15s."
exit 1


sudo sysctl -w net.ipv4.conf.rustainer0.rp_filter=0

echo
echo "Adicionando rota para 172.18.0.0/16 via rustainer0"
sudo ip route add 172.18.0.0/16 dev rustainer0
