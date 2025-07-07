for iface in all default rustainer0; do
  echo "$iface:"
  cat /proc/sys/net/ipv4/conf/$iface/rp_filter
done

sudo ip route add 172.18.0.0/16 dev rustainer0
