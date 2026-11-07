## Исправления

- **`system` и `dhcp://` в nameserver.** Конфиги OpenClash и nikki пишут `nameserver-policy: {'+.lan': system}`, а meow такие записи не понимал и с ними не запускался: «nameserver-policy entry '+.lan' has no valid nameservers». Теперь всё как в mihomo:
  - `system` и `system://` берут серверы из `/etc/resolv.conf` (на OpenWrt это dnsmasq, он отвечает на `.lan`);
  - `dhcp://<iface>` берёт серверы, полученные по DHCP, из `/tmp/resolv.conf.d/resolv.conf.auto`.
- **DNS отдаёт все адреса.** Раньше клиент получал одну A/AAAA-запись, даже если upstream вернул шесть. Теперь отдаются все, как в mihomo, до 8 штук. Каждый адрес попадает в `dns.nftset`.

## Скорость

- **Hysteria2:** quiche переиспользует буферы потоков вместо malloc на каждый пакет. Отдача через узел на BPI-R4: 372–472 → 506–524 Мбит/с, CPU на мегабит на ~10% меньше.

## Fixes

- **`system` / `dhcp://` nameservers** (OpenClash/nikki `'+.lan': system` used to stop meow from starting): expanded from `/etc/resolv.conf` and the DHCP lease file, as mihomo does.
- **The DNS listener answers every address** of the family (up to 8), not one; each goes to `dns.nftset`.
- **Hysteria2:** recycled quiche stream buffers; upload through a node 372–472 → 506–524 Mbit/s on the BPI-R4.
