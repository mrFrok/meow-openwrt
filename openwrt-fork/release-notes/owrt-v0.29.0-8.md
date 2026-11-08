## Из апстрима meow-rs

- **TUN:** сниффер — SNI из TLS, Host из HTTP и SNI из QUIC Initial для правил по доменам; `tun.stack: system` (TCP терминирует ядро) рядом с lwip.
- **REALITY:** гибридный ключ X25519MLKEM768 (`support-x25519mlkem768: true`). Xray 26+ без него клиента не пускает — теперь ошибка подсказывает, что включить.
- **DNS:** DNS-over-QUIC и DoH3 (`quic://`, `prefer-h3`; в full, кроме mips); исключительные совпадения `nameserver-policy`; UDP-апстримы по семейству адреса.
- **Конфиги:** `SAFE_PATHS`, `RULE-SET` внутри логических правил, health-check у провайдеров, `unified-delay`, список в `tun.inet4-address`, `xhttp` `mode: auto` как в Xray.
- **API:** ответы в формате mihomo для дашбордов — `/rules` с hitCount и отключением правил, `/storage`, CRUD правил.
- `.mrs` domain-set грузится с пиком памяти на ~6,5 МБ меньше; `meow convert-ruleset` — `.mrs` в текст.

## Скорость

- **Hysteria2, одно соединение:** ACK-only отправки объединяются (порт апстрима #866). BPI-R4, `connections: 1`: скачивание 470 → 532–560 Мбит/с, отдача 370 → 375–404. С 4 соединениями без изменений.

## Проверено

- BPI-R4: REALITY (~2 Гбит/с), hy2, сниффер TLS на обоих стеках.
- Официальный Xray 26.10.31: REALITY на Vision, gRPC и XHTTP, отпечатки chrome / firefox / safari.
- mips (big-endian) под qemu-mips: hy2 (включая UDP) и trojan против mihomo, REALITY против Xray, юнит-тесты крипто, REALITY, снифферов и hy2. После слияния mips не собирался — исправлено.

## From upstream meow-rs

- **TUN:** sniffer (TLS SNI, HTTP Host, QUIC Initial SNI) for domain rules; `tun.stack: system` (kernel-terminated TCP) next to lwip.
- **REALITY:** X25519MLKEM768 hybrid key share (`support-x25519mlkem768: true`). Xray 26+ rejects clients without it; the error now says so.
- **DNS:** DNS-over-QUIC and DoH3 (`quic://`, `prefer-h3`; full builds except mips); exclusive `nameserver-policy` matches; UDP upstreams bound per address family.
- **Config:** `SAFE_PATHS`, `RULE-SET` inside logic rules, provider health checks, `unified-delay`, list form of `tun.inet4-address`, Xray's `xhttp` `mode: auto`.
- **API:** mihomo response shapes for dashboards — `/rules` with hit counts and disabling, `/storage`, rule CRUD.
- `.mrs` domain sets load with ~6.5 MB lower peak memory; `meow convert-ruleset` turns `.mrs` into text.

## Speed

- **Hysteria2, single connection:** ACK-only flushes are coalesced (port of upstream #866). BPI-R4, `connections: 1`: download 470 → 532–560 Mbit/s, upload 370 → 375–404. Unchanged with 4 connections.

## Tested

- BPI-R4: REALITY (~2 Gbit/s), hy2, TLS sniffer on both stacks.
- Official Xray 26.10.31: REALITY over Vision, gRPC and XHTTP, chrome / firefox / safari fingerprints.
- mips (big-endian) under qemu-mips: hy2 (incl. UDP) and trojan against mihomo, REALITY against Xray, unit tests for crypto, REALITY, sniffers and hy2. mips failed to build after the merge — fixed.
