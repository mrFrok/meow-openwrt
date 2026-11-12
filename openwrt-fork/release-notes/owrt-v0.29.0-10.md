## Из апстрима meow-rs (52 коммита, по #953)

- **DNS:** `dns.listen` отвечает и по TCP с ограничением числа соединений и запросов; TUN перехватывает DNS по TCP; второй пул fake-IP для IPv6 (`fake-ip-range6`); `proxy-server-nameserver-policy`; правила ответов по типу запроса; DNS через UDP-прокси; исправлены `fake-ip-filter` с суффиксом на все домены и bootstrap шифрованных апстримов.
- **TUN / lwIP:** MTU учитывается для IPv4 и IPv6, сборка больших фрагментированных UDP-датаграмм, ограничение очереди TCP-accept, UDP-потоки и их задачи закрываются вместе с сессией.
- **Соединения:** адреса IPv4/IPv6 одного хоста перебираются с гонкой и общим дедлайном — зависший адрес больше не съедает весь таймаут.
- **Конфиги:** `ech-opts` на узле VLESS REALITY игнорируется, как в mihomo; `DOMAIN-REGEX` с запятыми в выражении; пустой `plugin` у Shadowsocks = обычный SS; недопустимые `max-connections` — ошибка конфига.
- **Hysteria2:** DNS-резолв сервера вынесен из-под блокировки соединения (не зависает, когда DNS сам идёт через этот же узел).
- В апстрим ушли и вернулись доработанными наши `redir-port` / TPROXY-листенеры / `routing-mark`, общий h2 для gRPC и XHTTP, `dns.nftset`, lwIP на libc malloc, окна приёма hy2.

## Поведение форка

- **`tproxy-port` остаётся как в mihomo** — настоящий TPROXY (TCP + UDP), правила за фронтендом. Апстрим решил оставить у себя этот ключ управляемым REDIRECT на loopback (meow-rs#901), так что это постоянное отличие форка. Апстримовская запись через `listeners:` (`type: tproxy`, `transparent: true`, `firewall: false`) тоже работает.
- **mimalloc не включён.** Апстрим включил его во всех musl-сборках; на роутере это 34–50 МБ RSS против ~5, поэтому здесь он остаётся опцией сборки (`alloc-mimalloc`).
- DNS-сервер по-прежнему отдаёт все адреса семейства (до 8 записей), как mihomo.

## Проверено

- BPI-R4 (TUN, оба стека): сниффер, DIRECT 5,8–6,2 Гбит/с, REALITY ~2 Гбит/с, hy2 — на уровне -9.
- TPROXY TCP/UDP и REDIRECT через `tproxy-port` / `redir-port` в netns (реальный и fake-IP адрес, `allow-lan: false`).

## From upstream meow-rs (52 commits, through #953)

- **DNS:** `dns.listen` also answers over TCP with bounded connections and queries; TUN intercepts TCP DNS; a second fake-IP pool for IPv6 (`fake-ip-range6`); `proxy-server-nameserver-policy`; response rules by query type; DNS over UDP-capable proxies; fixes for the all-domain `fake-ip-filter` suffix and encrypted bootstrap upstreams.
- **TUN / lwIP:** the MTU is honoured for IPv4 and IPv6, large fragmented UDP datagrams are reassembled, pending TCP accepts are bounded, UDP flows and their tasks end with the session.
- **Dialing:** a host's IPv4/IPv6 addresses are raced within one shared deadline — a stalled address no longer eats the whole timeout.
- **Config:** `ech-opts` on a VLESS REALITY node is ignored, as in mihomo; `DOMAIN-REGEX` payloads with commas; an empty Shadowsocks `plugin` means plain SS; unrepresentable `max-connections` is a config error.
- **Hysteria2:** the server's DNS lookup runs outside the connection lock (no stall when DNS itself goes through the same node).
- Our `redir-port` / TPROXY listeners / `routing-mark`, the shared h2 connection for gRPC and XHTTP, `dns.nftset`, lwIP on libc malloc and the hy2 receive windows went upstream and came back reviewed.

## Fork behaviour

- **`tproxy-port` stays mihomo's** — a real TPROXY inbound (TCP + UDP), rules owned by the front-end. Upstream decided to keep the key as managed REDIRECT on loopback (meow-rs#901), so this is a permanent difference of the fork. Upstream's `listeners:` form (`type: tproxy`, `transparent: true`, `firewall: false`) works here too.
- **mimalloc is not enabled.** Upstream turned it on for every musl build; on a router that is 34–50 MB RSS against ~5, so here it stays a build option (`alloc-mimalloc`).
- The DNS server still returns every address of the family (up to 8 records), as mihomo does.

## Tested

- BPI-R4 (TUN, both stacks): sniffer, DIRECT 5.8–6.2 Gbit/s, REALITY ~2 Gbit/s, hy2 — on par with -9.
- TPROXY TCP/UDP and REDIRECT through `tproxy-port` / `redir-port` in netns (real and fake-IP destinations, `allow-lan: false`).
